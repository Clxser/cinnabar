//! Native export creates isolated replay rendering inside the domain export worker.

use bevy::prelude::*;
use client_shell::{menu::MenuRuntime, ui_resources::UiPresentationResource};
use recording_studio::{
    export::{Cancellation, ExportFrame, ExportRequest, ExportTask, FrameSource},
    preview::{PreviewRequest, RasterPreview},
};
use render::replay::{ReplayReadback, ReplayRenderContext};

use super::composition::Composition;

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
    presentation: Res<UiPresentationResource>,
) {
    if let Some(request) = menu.take_studio_export_request() {
        if state
            .task
            .as_ref()
            .is_none_or(|task| !task.status().phase.busy())
        {
            state.task.take();
            let context = context.as_deref().cloned();
            let font = presentation.font_snapshot();
            match ExportTask::start(request, move || Ok(Frames::new(context, font))) {
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
    composition: Option<Composition>,
    raster: RasterPreview,
    readback: Option<ReplayReadback>,
    titles: super::titles::Titles,
}

impl Frames {
    /// No Bevy app crosses a thread boundary; its GPU scene is first constructed on frame evaluation.
    fn new(
        context: Option<ReplayRenderContext>,
        font: std::sync::Arc<assets::RuntimeFontCatalog>,
    ) -> Self {
        let mut titles = super::titles::Titles::new(context.clone());
        titles.set_font(font);
        Self {
            titles,
            composition: context
                .clone()
                .map(|context| Composition::new(context, false)),
            context,
            raster: RasterPreview::default(),
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
        Ok(())
    }

    /// Shared composition and title rendering complete before the sole export readback begins.
    fn frame(
        &mut self,
        request: &PreviewRequest,
        cancellation: &Cancellation,
    ) -> Result<ExportFrame, String> {
        let cancelled = || cancellation.cancelled();
        cancellation.check()?;
        let wait = || std::thread::sleep(std::time::Duration::from_millis(10));
        if let Some(composition) = &mut self.composition {
            let frame =
                composition.render(request, request.project.export.size, &cancelled, &wait)?;
            if frame.missing_target {
                return Err("A camera subject is missing at this export frame. Retarget or shorten its shot.".into());
            }
            let image = self
                .titles
                .render(request, frame.image, &cancelled, &wait)?;
            let size = image.target.size();
            if self.readback.is_none() {
                self.readback = Some(ReplayReadback::new(
                    self.context
                        .clone()
                        .ok_or("The export graphics device is unavailable.")?,
                    size,
                )?);
            }
            let rgba8 = self
                .readback
                .as_mut()
                .expect("export readback admitted")
                .frame(image, cancelled)?;
            return Ok(ExportFrame {
                size,
                rgba8,
                notice: frame.notice,
            });
        }
        let frame = self
            .raster
            .render(request, request.project.export.size, &cancelled)?;
        if !frame.effects.neutral() || self.titles.active(request)? {
            return Err("Timeline effects and titles need an initialized graphics device.".into());
        }
        Ok(ExportFrame {
            size: frame.size,
            rgba8: frame.rgba8,
            notice: None,
        })
    }
}
