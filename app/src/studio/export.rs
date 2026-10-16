//! Native export creates isolated replay rendering inside the domain export worker.

use bevy::prelude::*;
use client_shell::menu::MenuRuntime;
use recording_studio::{
    export::{Cancellation, ExportFrame, ExportRequest, ExportTask, FrameSource},
    playback::{CompiledTimeline, FramePlan},
    preview::{PreviewRequest, RasterPreview},
};
use render::replay::{FrameEffects, ReplayReadback, ReplayRenderContext};

use super::preview::scene::NativeScene;

mod admission;

#[derive(Resource, Default)]
pub(super) struct NativeExport {
    task: Option<ExportTask>,
}

/// The frame thread only transfers intent and publishes compact worker progress.
pub(super) fn update(
    mut state: ResMut<NativeExport>,
    mut menu: ResMut<MenuRuntime>,
    context: Option<Res<ReplayRenderContext>>,
) {
    if let Some(request) = menu.take_studio_export_request() {
        if state
            .task
            .as_ref()
            .is_none_or(|task| !task.status().phase.busy())
        {
            state.task.take();
            let context = context.as_deref().cloned();
            match ExportTask::start(request, move || Ok(Frames::new(context))) {
                Ok(task) => state.task = Some(task),
                Err(error) => {
                    menu.set_studio_export_status(recording_studio::export::ExportStatus {
                        phase: recording_studio::export::ExportPhase::Failed,
                        error: Some(error),
                        ..Default::default()
                    })
                }
            }
        }
    }
    if menu.take_studio_export_cancel() {
        if let Some(task) = &state.task {
            task.cancel();
        }
    }
    if let Some(task) = &state.task {
        let status = task.status();
        let terminal = !status.phase.busy();
        menu.set_studio_export_status(status);
        if terminal {
            state.task.take();
        }
    }
}

struct Frames {
    context: Option<ReplayRenderContext>,
    native: Option<NativeScene>,
    raster: RasterPreview,
    timeline: Option<CompiledTimeline>,
    plan: FramePlan,
    readback: Option<ReplayReadback>,
    effects: Option<FrameEffects>,
}

impl Frames {
    /// No Bevy app crosses a thread boundary; its GPU scene is first constructed on frame evaluation.
    fn new(context: Option<ReplayRenderContext>) -> Self {
        Self {
            effects: context.clone().map(FrameEffects::new),
            native: context.clone().map(NativeScene::new),
            context,
            raster: RasterPreview::default(),
            timeline: None,
            plan: FramePlan::default(),
            readback: None,
        }
    }
}

impl FrameSource for Frames {
    /// The complete timeline is checked before encoding, including features used late in the edit.
    fn prepare(
        &mut self,
        request: &ExportRequest,
        cancellation: &Cancellation,
    ) -> Result<(), String> {
        cancellation.check()?;
        admission::prepare(&request.project, self.context.is_some())?;
        self.timeline = Some(
            CompiledTimeline::new(request.project.clone()).map_err(|error| error.to_string())?,
        );
        Ok(())
    }

    /// Unsupported edits fail explicitly before a completed file can be published.
    fn frame(
        &mut self,
        request: &PreviewRequest,
        cancellation: &Cancellation,
    ) -> Result<ExportFrame, String> {
        let cancelled = || cancellation.cancelled();
        cancellation.check()?;
        if self.timeline.is_none() {
            self.timeline = Some(
                CompiledTimeline::new(request.project.clone())
                    .map_err(|error| error.to_string())?,
            );
        }
        let at = request.frame_time()?;
        self.timeline
            .as_ref()
            .expect("export timeline admitted")
            .sample(at, &mut self.plan, |_, _, _| None)
            .map_err(|error| error.to_string())?;
        if !self.plan.titles.is_empty() {
            return Err("Title rendering is not connected to export yet; remove titles to export this project.".into());
        }
        let wait = || std::thread::sleep(std::time::Duration::from_millis(10));
        let native = self
            .native
            .as_mut()
            .map(|native| native.render(request, request.project.export.size, &cancelled, &wait))
            .transpose()?
            .flatten();
        if let Some(native) = native {
            if native.missing_target {
                return Err("A camera subject is missing at this export frame. Retarget or shorten its shot.".into());
            }
            let size = native.image.target.size();
            if self.readback.is_none() {
                self.readback = Some(ReplayReadback::new(
                    self.context
                        .clone()
                        .ok_or("The native export device is unavailable.")?,
                    size,
                )?);
            }
            let rgba8 = self
                .readback
                .as_mut()
                .expect("export readback admitted")
                .frame(native.image, cancelled)?;
            return Ok(ExportFrame { size, rgba8, notice: Some("Native replay export uses Vanilla shading and the recorded actor draw coverage; HUD, hand, particles and block entities are omitted.".into()) });
        }
        if self.plan.shot.is_some() {
            return Err("Camera shots need a 3D replay source for export.".into());
        }
        let frame = self
            .raster
            .render(request, request.project.export.size, &cancelled)?;
        if !frame.effects.neutral() {
            let effects = self
                .effects
                .as_mut()
                .ok_or("Timeline effects need an initialized graphics device.")?;
            let started = std::time::Instant::now();
            let image = loop {
                cancellation.check()?;
                if let Some(image) = effects.raster(frame.size, &frame.rgba8, frame.effects, at)? {
                    break image;
                }
                if started.elapsed() >= std::time::Duration::from_secs(30) {
                    return Err("Export effects could not acquire an output image.".into());
                }
                wait();
            };
            if self.readback.is_none() {
                self.readback = Some(ReplayReadback::new(
                    self.context
                        .clone()
                        .ok_or("The export graphics device is unavailable.")?,
                    frame.size,
                )?);
            }
            let rgba8 = self
                .readback
                .as_mut()
                .expect("export readback admitted")
                .frame(image, cancelled)?;
            return Ok(ExportFrame {
                size: frame.size,
                rgba8,
                notice: None,
            });
        }
        Ok(ExportFrame {
            size: frame.size,
            rgba8: frame.rgba8,
            notice: None,
        })
    }
}
