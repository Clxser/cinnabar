//! Worker-owned raster preview inputs. Native replay surfaces plug into the same composition boundary.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

use crate::{
    Id, MediaTime, Project,
    decode::VideoDecoder,
    model::MediaKind,
    playback::{CompiledTimeline, FramePlan},
};

mod raster;

const MAX_PREVIEW_LAYERS: usize = 8;

#[derive(Clone)]
pub struct PreviewRequest {
    /// Document, edit revision and transport seek epoch; forward playback does not change this identity.
    pub epoch: [u64; 3],
    pub project: Arc<Project>,
    pub source_base: PathBuf,
    pub at: MediaTime,
}

impl PreviewRequest {
    /// Preview and export evaluate output frames on the same grid, including the final visible frame.
    pub fn frame_time(&self) -> Result<MediaTime, String> {
        let duration = self.project.duration();
        let at = MediaTime(self.at.0.min(duration.0.saturating_sub(1)));
        let frame = self
            .project
            .export
            .fps
            .frame_at(at)
            .map_err(|error| error.to_string())?;
        self.project
            .export
            .fps
            .frame_time(frame)
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreviewStatus {
    pub loading: bool,
    pub ready: bool,
    pub at: MediaTime,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub camera: Option<CameraPreview>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CameraPreview {
    pub epoch: [u64; 3],
    pub source: Id,
    pub source_time: MediaTime,
    /// Rig pose before transient timeline shake/zoom, so creating a shot cannot apply an effect twice.
    pub pose: crate::camera::CameraPose,
    pub local_actor: u64,
    pub targets: Arc<[crate::camera::CameraTarget]>,
}

pub struct RasterFrame {
    pub size: [u32; 2],
    pub rgba8: Arc<[u8]>,
    pub notice: Option<String>,
}

pub struct RasterPreview {
    timeline: Option<CompiledTimeline>,
    source_base: PathBuf,
    decoders: BTreeMap<Id, VideoDecoder>,
    failures: BTreeMap<Id, String>,
    epoch: Option<[u64; 3]>,
    plan: FramePlan,
}

impl Default for RasterPreview {
    fn default() -> Self {
        Self {
            timeline: None,
            source_base: PathBuf::new(),
            decoders: BTreeMap::new(),
            failures: BTreeMap::new(),
            epoch: None,
            plan: FramePlan::default(),
        }
    }
}

impl RasterPreview {
    /// Evaluates the saved timeline, reusing indexes and adjacent video decoding on a preview worker.
    pub fn render(
        &mut self,
        request: &PreviewRequest,
        maximum: [u32; 2],
        cancelled: impl Fn() -> bool,
    ) -> Result<RasterFrame, String> {
        if self.epoch != Some(request.epoch) {
            self.failures.clear();
            self.epoch = Some(request.epoch);
        }
        if self
            .timeline
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old.project(), &request.project))
            || self.source_base != request.source_base
        {
            self.timeline = Some(
                CompiledTimeline::new(Arc::clone(&request.project))
                    .map_err(|error| error.to_string())?,
            );
            self.source_base.clone_from(&request.source_base);
        }
        let timeline = self
            .timeline
            .as_ref()
            .ok_or("Preview timeline is unavailable.")?;
        let project = timeline.project();
        let at = request.frame_time()?;
        timeline
            .sample(at, &mut self.plan, |_, _, _| None)
            .map_err(|error| error.to_string())?;
        if self.plan.visuals.len() > MAX_PREVIEW_LAYERS {
            return Err("Preview supports at most eight simultaneous visual layers.".into());
        }
        let active = self
            .plan
            .visuals
            .iter()
            .map(|visual| visual.source)
            .collect::<BTreeSet<_>>();
        self.decoders.retain(|id, _| active.contains(id));
        self.failures.retain(|id, _| active.contains(id));
        let size = fit_size(project.export.size, maximum)?;
        let mut pending = Vec::new();
        if !self.plan.audio.is_empty() {
            pending.push("audio");
        }
        if !project.shots.is_empty() {
            pending.push("camera changes");
        }
        if !self.plan.titles.is_empty() {
            pending.push("titles");
        }
        if !project.effects.is_empty()
            || project
                .transitions
                .iter()
                .any(|transition| transition.kind == crate::model::TransitionKind::DipToBlack)
        {
            pending.push("effects");
        }
        let notice =
            (!pending.is_empty()).then(|| format!("Not previewed yet: {}.", pending.join(", ")));
        let mut decode = |visual: &crate::playback::VisualSample| {
            if cancelled() {
                return Err("Preview cancelled.".into());
            }
            let source = project
                .sources
                .iter()
                .find(|source| source.id == visual.source)
                .ok_or("Preview source is missing.")?;
            if source.kind == MediaKind::Replay {
                return Err("Native replay camera preview is still being connected.".into());
            }
            if !matches!(source.kind, MediaKind::Video | MediaKind::Image) {
                return Err("This visual source is not video or an image.".into());
            }
            if let Some(error) = self.failures.get(&source.id) {
                return Err(error.clone());
            }
            let path = if source.path.is_absolute() {
                source.path.clone()
            } else {
                self.source_base.join(&source.path)
            };
            let decoded_size = fit_size(source.size, maximum)?;
            if self
                .decoders
                .get(&source.id)
                .is_none_or(|decoder| !decoder.matches(&path, decoded_size, project.export.fps))
            {
                self.decoders.insert(
                    source.id,
                    VideoDecoder::new(&path, decoded_size, project.export.fps)?,
                );
            }
            let source_at = if source.kind == MediaKind::Image {
                MediaTime::ZERO
            } else {
                visual.time
            };
            let frame = self
                .decoders
                .get_mut(&source.id)
                .ok_or("Preview decoder is missing.")?
                .frame(source_at, &cancelled);
            let frame = match frame {
                Ok(frame) => frame,
                Err(error) => {
                    if !cancelled() {
                        self.failures.insert(source.id, error.clone());
                    }
                    return Err(error);
                }
            };
            Ok(frame)
        };
        if let [visual] = &self.plan.visuals[..]
            && visual.mix.is_none()
            && visual.framing == crate::model::Framing::default()
        {
            let frame = decode(visual)?;
            if frame.size == size && frame.rgba8.chunks_exact(4).all(|pixel| pixel[3] == 255) {
                if cancelled() {
                    return Err("Preview cancelled.".into());
                }
                return Ok(RasterFrame {
                    size,
                    rgba8: frame.rgba8,
                    notice,
                });
            }
        }
        let mut composed = raster::Raster::new(size)?;
        let mut mixed = BTreeSet::new();
        for visual in &self.plan.visuals {
            if let Some(group) = visual.mix {
                if !mixed.insert(group) {
                    continue;
                }
                let mut dissolve = raster::Raster::mix(size)?;
                for member in self
                    .plan
                    .visuals
                    .iter()
                    .filter(|member| member.mix == Some(group))
                {
                    dissolve.overlay(&decode(member)?, member.framing, true, &cancelled)?;
                }
                composed.overlay_mix(dissolve, &cancelled)?;
            } else {
                composed.overlay(&decode(visual)?, visual.framing, false, &cancelled)?;
            }
        }
        if cancelled() {
            return Err("Preview cancelled.".into());
        }
        Ok(RasterFrame {
            size,
            rgba8: composed.finish(&cancelled)?,
            notice,
        })
    }
}

/// Preserves aspect ratio without exceeding the caller's admitted preview extent or upscaling media.
pub fn fit_size(size: [u32; 2], maximum: [u32; 2]) -> Result<[u32; 2], String> {
    if size.contains(&0) || maximum.contains(&0) {
        return Err("Preview dimensions must be nonzero.".into());
    }
    let scale = (f64::from(maximum[0]) / f64::from(size[0]))
        .min(f64::from(maximum[1]) / f64::from(size[1]))
        .min(1.0);
    Ok(size.map(|side| (f64::from(side) * scale).floor().max(1.0) as u32))
}
