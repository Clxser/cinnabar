//! Native replay sources use their own renderer; raster video never borrows the live world.

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use recording_scene::{
    actors::ActorCoverage, playback::ScenePlayback, scene::DEFAULT_RESIDENT_BYTES,
};
use recording_studio::{
    Id, MediaTime,
    camera::CameraPose,
    model::{Framing, MediaKind},
    playback::{CompiledTimeline, FramePlan},
    preview::{CameraPreview, PreviewRequest, fit_size},
};
use render::replay::{FrameEffects, ReplayRenderContext, ReplayRenderer};
use render_scene::ui_preview::PreviewImage;

pub(in crate::studio) struct NativeScene {
    context: ReplayRenderContext,
    timeline: Option<CompiledTimeline>,
    plan: FramePlan,
    source: Option<Source>,
    effects: FrameEffects,
}

pub(in crate::studio) struct NativeFrame {
    pub image: Arc<PreviewImage>,
    pub notice: Option<String>,
    pub camera: CameraPreview,
    pub missing_target: bool,
}

struct Source {
    id: Id,
    path: PathBuf,
    document: u64,
    epoch: Option<[u64; 2]>,
    playback: ScenePlayback,
    renderer: ReplayRenderer,
    base: Option<BaseFrame>,
}

struct BaseFrame {
    at: MediaTime,
    pose: CameraPose,
    image: Arc<PreviewImage>,
}

impl NativeScene {
    /// Scene and GPU contexts are created only on the existing preview worker.
    pub(in crate::studio) fn new(context: ReplayRenderContext) -> Self {
        Self {
            effects: FrameEffects::new(context.clone()),
            context,
            timeline: None,
            plan: FramePlan::default(),
            source: None,
        }
    }

    /// Returns `None` for a raster-only frame; mixed replay composition remains explicitly unsupported.
    pub(in crate::studio) fn render(
        &mut self,
        request: &PreviewRequest,
        maximum: [u32; 2],
        cancelled: &impl Fn() -> bool,
        wait: &impl Fn(),
    ) -> Result<Option<NativeFrame>, String> {
        if self
            .timeline
            .as_ref()
            .is_none_or(|timeline| !Arc::ptr_eq(timeline.project(), &request.project))
        {
            self.timeline = Some(
                CompiledTimeline::new(Arc::clone(&request.project))
                    .map_err(|error| error.to_string())?,
            );
        }
        let timeline = self
            .timeline
            .as_ref()
            .ok_or("The replay timeline is unavailable.")?;
        let at = request.frame_time()?;
        timeline
            .sample(at, &mut self.plan, |_, _, _| None)
            .map_err(|error| error.to_string())?;
        if !self.plan.visuals.iter().any(|visual| {
            request
                .project
                .sources
                .iter()
                .any(|source| source.id == visual.source && source.kind == MediaKind::Replay)
        }) {
            self.source = None;
            return Ok(None);
        }
        let [visual] = &self.plan.visuals[..] else {
            return Err(
                "Combining a native replay with another visual layer is not connected yet.".into(),
            );
        };
        let visual = *visual;
        if visual.mix.is_some() || visual.framing != Framing::default() {
            return Err(
                "Native replay crops, opacity and transitions are not connected yet.".into(),
            );
        }
        let media = request
            .project
            .sources
            .iter()
            .find(|source| source.id == visual.source)
            .ok_or("The replay source is missing.")?;
        let path = if media.path.is_absolute() {
            media.path.clone()
        } else {
            request.source_base.join(&media.path)
        };
        let size = fit_size(request.project.export.size, maximum)?;
        if self.source.as_ref().is_none_or(|source| {
            source.id != media.id
                || source.path != path
                || source.document != request.epoch[0]
                || source.renderer.size() != size
        }) {
            self.source = None;
            if cancelled() {
                return Err("Preview cancelled.".into());
            }
            let playback = ScenePlayback::open(&path, DEFAULT_RESIDENT_BYTES)
                .map_err(|error| format!("Cannot open native replay: {error}"))?;
            let renderer = ReplayRenderer::new(self.context.clone(), size)?;
            self.source = Some(Source {
                id: media.id,
                path,
                document: request.epoch[0],
                epoch: None,
                playback,
                renderer,
                base: None,
            });
        }
        let source = self
            .source
            .as_mut()
            .ok_or("The replay renderer is unavailable.")?;
        if visual.time >= source.playback.duration() {
            return Err(
                "This frame lies past the native scene tracks' common recorded duration.".into(),
            );
        }
        let source_epoch = [request.epoch[0], request.epoch[2]];
        let frame = if source.epoch != Some(source_epoch)
            || source
                .playback
                .position()
                .is_none_or(|previous| previous > visual.time)
        {
            source.playback.seek(visual.time, cancelled)
        } else {
            source.playback.advance_to(visual.time, cancelled)
        }
        .map_err(|error| format!("Native replay admission failed: {error}"))?;
        source.epoch = Some(source_epoch);
        timeline
            .sample(at, &mut self.plan, |id, reference, time| {
                if id != visual.source || time != visual.time {
                    return None;
                }
                let target = frame.targets.as_ref()?.target(reference.runtime_id)?;
                target.matches(reference).then_some(target.pose)
            })
            .map_err(|error| error.to_string())?;
        let missing_target = self.plan.shot.is_some() && self.plan.camera.is_none();
        let pose = self
            .plan
            .camera
            .unwrap_or_else(|| self.plan.effects.camera_pose(frame.view.pose, at));
        let mut notices = Vec::new();
        if missing_target {
            notices.push("Camera subject unavailable: showing the recorded view. Retarget or shorten the shot.");
        }
        notices.push(
            "3D preview uses Vanilla shading and omits HUD, hand, particles and block entities.",
        );
        if frame.actors.sample.coverage == ActorCoverage::OriginalView {
            notices.push(
                "Actors cover the recorded view only; other cameras can expose missing actors.",
            );
        }
        if frame.targets.is_none() {
            notices.push("This take has no received-actor target track; actor tracking needs a newer recording.");
        }
        if !self.plan.audio.is_empty() {
            notices.push("Audio is not previewed yet.");
        }
        let started = Instant::now();
        let image = if let Some(base) = source
            .base
            .as_ref()
            .filter(|base| !frame.temporal_reset && base.at == visual.time && base.pose == pose)
        {
            Arc::clone(&base.image)
        } else {
            source.base = None;
            if let Err(error) = source.renderer.apply(&frame, pose) {
                source.playback.invalidate();
                source.epoch = None;
                return Err(error);
            }
            let image = loop {
                if cancelled() {
                    return Err("Preview cancelled.".into());
                }
                match source.renderer.draw() {
                    Ok(Some(image)) => break image,
                    Ok(None) => {}
                    Err(error) => {
                        source.playback.invalidate();
                        source.epoch = None;
                        return Err(error);
                    }
                }
                if started.elapsed() >= Duration::from_secs(30) {
                    return Err("The native replay scene could not finish GPU preparation. Seek again to retry.".into());
                }
                wait();
            };
            source.base = Some(BaseFrame {
                at: visual.time,
                pose,
                image: Arc::clone(&image),
            });
            image
        };
        let image = loop {
            if cancelled() {
                return Err("Preview cancelled.".into());
            }
            if let Some(composed) =
                self.effects
                    .native(Arc::clone(&image), self.plan.effects, at)?
            {
                break composed;
            }
            if started.elapsed() >= Duration::from_secs(30) {
                return Err(
                    "The effect preview could not finish. Pause playback and seek again.".into(),
                );
            }
            wait();
        };
        Ok(Some(NativeFrame {
            image,
            missing_target,
            notice: (!notices.is_empty()).then(|| notices.join(" ")),
            camera: CameraPreview {
                epoch: request.epoch,
                source: visual.source,
                source_time: visual.time,
                pose: self.plan.base_camera.unwrap_or(frame.view.pose),
                local_actor: source.playback.local_actor(),
                targets: frame
                    .targets
                    .as_ref()
                    .map_or_else(|| Arc::from([]), |frame| Arc::clone(&frame.targets)),
            },
        }))
    }
}
