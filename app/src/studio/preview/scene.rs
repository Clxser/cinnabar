//! Native replay sources use their own renderer; raster video never borrows the live world.

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use recording_scene::{actors::ActorCoverage, playback::ScenePlayback};
use recording_studio::{
    Id, MediaTime,
    camera::CameraPose,
    playback::{CompiledTimeline, FramePlan, VisualSample},
    preview::{CameraPreview, PreviewRequest, WorldLabelCoverage, fit_size},
};
use render::replay::{ReplayRenderContext, ReplayRenderer};
use render_scene::ui_preview::PreviewImage;

pub(in crate::studio) struct NativeScene {
    context: ReplayRenderContext,
    timeline: Option<CompiledTimeline>,
    plan: FramePlan,
    source: Option<Source>,
    resident_bytes: usize,
}

pub(in crate::studio) struct NativeFrame {
    pub image: Arc<PreviewImage>,
    pub notices: Vec<&'static str>,
    pub camera: CameraPreview,
    pub missing_target: bool,
}

struct Source {
    id: Id,
    path: PathBuf,
    document: u64,
    content: Option<[u8; 32]>,
    epoch: Option<[u64; 2]>,
    playback: ScenePlayback,
    renderer: ReplayRenderer,
    base: Option<BaseFrame>,
}

struct BaseFrame {
    at: MediaTime,
    pose: CameraPose,
    image: Arc<PreviewImage>,
    visibility: [bool; 3],
    spectated: Option<recording_studio::camera::CameraTargetRef>,
}

impl NativeScene {
    /// Adjacent non-overlapping cuts of one replay can reuse its admitted world and GPU pipelines.
    pub(in crate::studio) fn source_id(&self) -> Option<Id> {
        self.source.as_ref().map(|source| source.id)
    }
    /// Scene and GPU contexts are created only on the existing preview worker.
    pub(in crate::studio) fn new(context: ReplayRenderContext, resident_bytes: usize) -> Self {
        Self {
            resident_bytes,
            context,
            timeline: None,
            plan: FramePlan::default(),
            source: None,
        }
    }

    /// Changes a composition's share without rebuilding GPU resources or restarting source clocks.
    pub(in crate::studio) fn set_resident_limit(
        &mut self,
        maximum_bytes: usize,
    ) -> Result<(), String> {
        if let Some(source) = &mut self.source {
            source
                .playback
                .set_terrain_budget(maximum_bytes)
                .map_err(|error| format!("Native replay memory admission failed: {error}"))?;
        }
        self.resident_bytes = maximum_bytes;
        Ok(())
    }

    /// Renders one isolated visual before crop, blending, post effects and titles.
    /// Actor rigs resolve against this visual's source, retaining independent crossfade cursors.
    pub(in crate::studio) fn render_visual(
        &mut self,
        request: &PreviewRequest,
        visual: VisualSample,
        media: &recording_studio::model::MediaSource,
        maximum: [u32; 2],
        cancelled: &impl Fn() -> bool,
        wait: &impl Fn(),
    ) -> Result<NativeFrame, String> {
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
        let path = if media.path.is_absolute() {
            media.path.clone()
        } else {
            request.source_base.join(&media.path)
        };
        let size = fit_size(media.size, maximum)?;
        if self.source.as_ref().is_none_or(|source| {
            source.id != media.id
                || source.path != path
                || source.document != request.epoch[0]
                || source.content != media.sha256
                || source.renderer.size() != size
        }) {
            self.source = None;
            if cancelled() {
                return Err("Preview cancelled.".into());
            }
            let playback = ScenePlayback::open(&path, self.resident_bytes)
                .map_err(|error| format!("Cannot open native replay: {error}"))?;
            let renderer = ReplayRenderer::new(self.context.clone(), size)?;
            self.source = Some(Source {
                id: media.id,
                path,
                document: request.epoch[0],
                content: media.sha256,
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
        let frame = if (source.epoch != Some(source_epoch)
            && source.playback.position() != Some(visual.time))
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
            .sample(at, &mut self.plan, |_, reference, _| {
                let target = frame.targets.as_ref()?.target(reference.runtime_id)?;
                target.matches(reference).then_some(target.pose)
            })
            .map_err(|error| error.to_string())?;
        let missing_target = self.plan.shot.is_some() && self.plan.camera.is_none();
        // Inspection poses are already final camera views; applying effects again would move
        // the observer while dragging a projected handle.
        let pose = request.camera_view.map_or_else(
            || {
                self.plan
                    .camera
                    .unwrap_or_else(|| self.plan.effects.camera_pose(frame.view.pose, at))
            },
            |view| view.pose,
        );
        let source_view = request.camera_view.is_none() && self.plan.base_camera.is_none();
        let visibility = [source_view, self.plan.hide_hud, self.plan.hide_hand];
        let spectated = request
            .camera_view
            .is_none()
            .then_some(self.plan.spectated)
            .flatten();
        let mut notices = Vec::new();
        if missing_target {
            let missing_eye = self
                .plan
                .shot
                .and_then(|id| request.project.shots.iter().find(|shot| shot.id == id))
                .and_then(|shot| shot.spectated_actor())
                .and_then(|reference| {
                    frame
                        .targets
                        .as_ref()?
                        .target(reference.runtime_id)
                        .filter(|target| target.matches(reference))
                })
                .is_some_and(|target| target.pose.eye_offset.is_none());
            notices.push(if missing_eye {
                "This take has no native eye view for this subject. Use Track, Follow or Orbit; remote POV cannot be reconstructed from a body centre."
            } else {
                "Camera subject unavailable: showing the recorded view. Retarget or shorten the shot."
            });
        }
        if let Some(visuals) = &frame.visuals {
            if visuals.particles.received.is_none() {
                notices.push(
                    "This older take retains source-view particle visibility and billboard facing.",
                );
            }
            if let Some(coverage) = visuals.block_coverage {
                if !coverage.artwork_available {
                    notices
                        .push("Block-model artwork was unavailable while recording this sample.");
                }
                if coverage.pending_columns > 0 {
                    notices.push(
                        "Received block-model coverage was still warming up at this source moment.",
                    );
                }
                if coverage.cache_limit || coverage.submission_limit || coverage.vertex_limit {
                    notices.push(
                        "Block-model coverage reached a native preparation limit while recording.",
                    );
                }
            } else {
                notices.push("This older take retains source-view block-model visibility.");
            }
            if visuals.atmosphere.is_none() {
                notices.push("This older take has no prepared weather and screen-overlay inputs.");
            }
            if let Some(items) = &visuals.items {
                if items.coverage.model_failures > 0 {
                    notices.push(
                        "Some native item models were unavailable while recording this sample.",
                    );
                }
                if items.coverage.instance_limit || items.coverage.rope_limit {
                    notices.push("Item or rope capture reached its native preparation limit.");
                }
                if items.coverage.terrain_pending {
                    notices.push(
                        "Falling-block terrain visibility was still awaiting source publication.",
                    );
                }
            } else {
                notices.push("This take has no prepared item, falling-block or rope track.");
            }
            if request.project.export.include_recorded_hud && !visuals.hud_enabled {
                notices.push("HUD and chat were not recorded for this take.");
            } else if request.project.export.include_recorded_hud
                && (!source_view || self.plan.hide_hud)
            {
                notices.push("Recorded HUD and chat are hidden for this camera shot.");
            }
            if !visuals.hand_enabled {
                notices.push("The first-person hand was not recorded for this take.");
            } else if !source_view || self.plan.hide_hand {
                notices.push("The recorded first-person hand is hidden for this camera shot.");
            }
        } else {
            notices
                .push("This older take has no prepared particle, block model, hand or HUD track.");
        }
        if frame.actors.sample.coverage == ActorCoverage::OriginalView {
            notices.push(
                "Actors cover the recorded view only; other cameras can expose missing actors.",
            );
        }
        if frame.targets.is_none() {
            notices.push("This take has no received-actor target track; actor tracking needs a newer recording.");
        }
        let world_labels = if frame
            .visuals
            .as_ref()
            .is_none_or(|visuals| visuals.nametags.is_none())
        {
            WorldLabelCoverage::Unavailable
        } else {
            WorldLabelCoverage::RecordedViewer
        };
        if let Some(notice) = world_labels.notice(!source_view) {
            notices.push(notice);
        }
        if !source_view {
            notices.push("Custom cameras can show only terrain and entities received during this take. Move back into recorded coverage or record a new take near missing content.");
        }
        let started = Instant::now();
        let image = if let Some(base) = source.base.as_ref().filter(|base| {
            !frame.temporal_reset
                && base.at == visual.time
                && base.pose == pose
                && base.visibility == visibility
                && base.spectated == spectated
        }) {
            Arc::clone(&base.image)
        } else {
            source.base = None;
            if let Err(error) = source.renderer.apply_view(
                &frame,
                pose,
                source_view,
                self.plan.hide_hud,
                self.plan.hide_hand,
                spectated,
            ) {
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
                spectated,
                at: visual.time,
                pose,
                image: Arc::clone(&image),
                visibility,
            });
            image
        };
        Ok(NativeFrame {
            image,
            missing_target,
            notices,
            camera: CameraPreview {
                world_labels,
                epoch: request.epoch,
                source: visual.source,
                source_time: visual.time,
                pose: self.plan.base_camera.unwrap_or(frame.view.pose),
                render_pose: pose,
                interactive: true,
                viewport: [0.0, 0.0, 1.0, 1.0],
                local_actor: source.playback.local_actor(),
                targets: frame
                    .targets
                    .as_ref()
                    .map_or_else(|| Arc::from([]), |frame| Arc::clone(&frame.targets)),
            },
        })
    }
}
