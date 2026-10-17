//! Preview and export share retained per-clip media contexts and one GPU composition order.

use super::preview::scene::NativeScene;
use recording_studio::{
    Id,
    decode::VideoDecoder,
    model::MediaKind,
    playback::{CompiledTimeline, FramePlan, MAX_VISUAL_LAYERS},
    preview::{CameraPreview, MAX_VISUAL_FRAME_BYTES, PreviewRequest, fit_size},
};
use render::replay::{
    CompositeInput, CompositeLayer, FrameCompositor, FrameEffects, ReplayRenderContext,
};
use render_scene::ui_preview::PreviewImage;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

struct RasterSource {
    source: Id,
    path: PathBuf,
    epoch: [u64; 2],
    decoder: VideoDecoder,
}

pub(super) struct Composition {
    context: ReplayRenderContext,
    timeline: Option<CompiledTimeline>,
    source_indices: BTreeMap<Id, usize>,
    plan: FramePlan,
    native: BTreeMap<Id, NativeScene>,
    native_budget: usize,
    raster: BTreeMap<Id, RasterSource>,
    compositor: FrameCompositor,
    effects: FrameEffects,
    interactive: bool,
}

pub(super) struct ComposedFrame {
    pub image: Arc<PreviewImage>,
    pub camera: Option<CameraPreview>,
    pub notice: Option<String>,
    pub missing_target: bool,
    pub native: bool,
}

impl Composition {
    /// Each owning worker retains its own cursors and GPU scratch while sharing only device handles.
    pub fn new(context: ReplayRenderContext, interactive: bool) -> Self {
        Self {
            compositor: FrameCompositor::new(context.clone()),
            effects: FrameEffects::new(context.clone()),
            context,
            timeline: None,
            source_indices: BTreeMap::new(),
            plan: FramePlan::default(),
            native: BTreeMap::new(),
            native_budget: recording_scene::scene::DEFAULT_RESIDENT_BYTES,
            raster: BTreeMap::new(),
            interactive,
        }
    }

    /// Counts the actual decoder backends, including software fallback in mixed native/raster frames.
    pub fn decoder_counts(&self) -> (usize, usize) {
        let hardware = self
            .raster
            .values()
            .filter(|source| source.decoder.hardware())
            .count();
        (hardware, self.raster.len() - hardware)
    }

    /// Camera motion is evaluated once per source; crop, transition groups and post effects follow it.
    pub fn render(
        &mut self,
        request: &PreviewRequest,
        maximum: [u32; 2],
        cancelled: &impl Fn() -> bool,
        wait: &impl Fn(),
    ) -> Result<ComposedFrame, String> {
        if cancelled() {
            return Err("Composition cancelled.".into());
        }
        if self
            .timeline
            .as_ref()
            .is_none_or(|timeline| !Arc::ptr_eq(timeline.project(), &request.project))
        {
            self.timeline = Some(
                CompiledTimeline::new(request.project.clone())
                    .map_err(|error| error.to_string())?,
            );
            self.source_indices = request
                .project
                .sources
                .iter()
                .enumerate()
                .map(|(index, source)| (source.id, index))
                .collect();
        }
        let at = request.frame_time()?;
        self.timeline
            .as_ref()
            .expect("composition timeline admitted")
            .sample(at, &mut self.plan, |_, _, _| None)
            .map_err(|error| error.to_string())?;
        if self.plan.visuals.len() > MAX_VISUAL_LAYERS {
            return Err("The frame exceeds its visual layer budget.".into());
        }
        let active_native: BTreeSet<_> = self
            .plan
            .visuals
            .iter()
            .filter(|visual| {
                self.source_indices
                    .get(&visual.source)
                    .is_some_and(|&index| request.project.sources[index].kind == MediaKind::Replay)
            })
            .map(|visual| visual.clip)
            .collect();
        let active_raster: BTreeSet<_> = self
            .plan
            .visuals
            .iter()
            .filter(|visual| {
                self.source_indices
                    .get(&visual.source)
                    .is_some_and(|&index| request.project.sources[index].kind != MediaKind::Replay)
            })
            .map(|visual| visual.clip)
            .collect();
        let budget = recording_scene::scene::DEFAULT_RESIDENT_BYTES / active_native.len().max(1);
        if budget < self.native_budget {
            self.native.clear();
            self.native_budget = budget;
        }
        self.native.retain(|clip, _| active_native.contains(clip));
        if active_native.is_empty() {
            self.native_budget = recording_scene::scene::DEFAULT_RESIDENT_BYTES;
        }
        self.raster.retain(|clip, _| active_raster.contains(clip));
        if self.plan.shot.is_some() && active_native.is_empty() {
            return Err(
                "Camera shots need a 3D replay clip throughout their timeline range.".into(),
            );
        }
        let size = fit_size(request.project.export.size, maximum)?;
        let mut frame_bytes = 0_usize;
        for visual in &self.plan.visuals {
            let source = self
                .source_indices
                .get(&visual.source)
                .and_then(|&index| request.project.sources.get(index))
                .ok_or("A composed media source is missing.")?;
            let bytes = recording_studio::video::frame_byte_len(fit_size(source.size, size)?)?;
            frame_bytes = frame_bytes
                .checked_add(bytes)
                .filter(|bytes| *bytes <= MAX_VISUAL_FRAME_BYTES)
                .ok_or("Visual layers exceed their combined frame byte budget.")?;
        }
        let motion = self.plan.effects.image_motion(at);
        let mut layers = Vec::with_capacity(self.plan.visuals.len());
        let mut camera = None;
        let mut missing_target = false;
        let mut notices = Vec::new();
        for visual in &self.plan.visuals {
            if cancelled() {
                return Err("Composition cancelled.".into());
            }
            let source = self
                .source_indices
                .get(&visual.source)
                .and_then(|&index| request.project.sources.get(index))
                .ok_or("A composed media source is missing.")?;
            let (input, motion) = if source.kind == MediaKind::Replay {
                let native = self
                    .native
                    .entry(visual.clip)
                    .or_insert_with(|| NativeScene::new(self.context.clone(), self.native_budget));
                let frame =
                    native.render_visual(request, *visual, source, size, cancelled, wait)?;
                missing_target |= frame.missing_target;
                camera = Some(frame.camera);
                for notice in frame.notices {
                    if !notices.contains(&notice) {
                        notices.push(notice);
                    }
                }
                (CompositeInput::Native(frame.image), [1.0, 0.0, 0.0])
            } else {
                if !matches!(source.kind, MediaKind::Video | MediaKind::Image) {
                    return Err("A visual layer is not a replay, video or image.".into());
                }
                let path = if source.path.is_absolute() {
                    source.path.clone()
                } else {
                    request.source_base.join(&source.path)
                };
                let decoded_size = fit_size(source.size, size)?;
                let fps = source.frame_rate.unwrap_or(request.project.export.fps);
                let epoch = [request.epoch[0], request.epoch[2]];
                if self.raster.get(&visual.clip).is_none_or(|old| {
                    old.source != source.id
                        || old.path != path
                        || !old.decoder.matches(&path, decoded_size, fps)
                }) {
                    let decoder = if self.interactive {
                        VideoDecoder::interactive(&path, decoded_size, fps)?
                    } else {
                        VideoDecoder::new(&path, decoded_size, fps)?
                    };
                    self.raster.insert(
                        visual.clip,
                        RasterSource {
                            source: source.id,
                            path,
                            epoch,
                            decoder,
                        },
                    );
                }
                let raster = self
                    .raster
                    .get_mut(&visual.clip)
                    .expect("raster decoder admitted");
                if raster.epoch != epoch {
                    raster.decoder.seek();
                    raster.epoch = epoch;
                }
                let time = if source.kind == MediaKind::Image {
                    recording_studio::MediaTime::ZERO
                } else {
                    visual.time
                };
                (
                    CompositeInput::Raster(raster.decoder.frame(time, cancelled)?),
                    motion,
                )
            };
            layers.push(CompositeLayer {
                clip: visual.clip,
                input,
                framing: visual.framing,
                mix: visual.mix,
                motion,
            });
        }
        let started = Instant::now();
        let image = loop {
            if cancelled() {
                return Err("Composition cancelled.".into());
            }
            if let Some(image) = self.compositor.render(size, &layers, cancelled)? {
                break image;
            }
            if started.elapsed() >= Duration::from_secs(30) {
                return Err(
                    "Composed frame outputs are still leased. Pause and seek again.".into(),
                );
            }
            wait();
        };
        // All raster layers received image motion and all replay layers received camera motion above.
        let image = loop {
            if cancelled() {
                return Err("Composition cancelled.".into());
            }
            if let Some(image) =
                self.effects
                    .composed(image.clone(), self.plan.effects, at, true)?
            {
                break image;
            }
            if started.elapsed() >= Duration::from_secs(30) {
                return Err("Composed frame effects could not acquire an output image.".into());
            }
            wait();
        };
        if self.interactive && !self.plan.audio.is_empty() {
            let audio = "Audio is not previewed yet.";
            if !notices.contains(&audio) {
                notices.push(audio);
            }
        }
        Ok(ComposedFrame {
            image,
            camera,
            notice: (!notices.is_empty()).then(|| notices.join(" ")),
            missing_target,
            native: !active_native.is_empty(),
        })
    }
}
