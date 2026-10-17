//! The bounded worker-decoded library sheet occupies one independent native UI page.

use super::UiPresentationRuntime;
use render_model::{UI_STUDIO_LIBRARY_PAGE_OFFSET, UI_STUDIO_PREVIEW_MAX_SIZE, UiTexturePage};
use std::sync::Arc;

impl UiPresentationRuntime {
    /// Reuses immutable thumbnail pixels across frames and resolves UVs against the current page layout.
    pub(super) fn prepare_studio_library(
        &mut self,
        workflow: &mut launcher::menu::studio::Workflow,
    ) {
        let changed = match (&workflow.thumbnail_sheet, &self.studio_library_pixels) {
            (Some(next), Some(old)) => !Arc::ptr_eq(&next.rgba8, old),
            (None, None) => false,
            _ => true,
        };
        if changed {
            self.studio_library_pixels = workflow
                .thumbnail_sheet
                .as_ref()
                .map(|sheet| sheet.rgba8.clone());
            self.studio_library_page = workflow
                .thumbnail_sheet
                .as_ref()
                .filter(|sheet| {
                    sheet
                        .size
                        .iter()
                        .zip(UI_STUDIO_PREVIEW_MAX_SIZE)
                        .all(|(&side, limit)| side > 0 && side <= limit)
                })
                .and_then(|sheet| UiTexturePage::owned(sheet.size, sheet.rgba8.clone()).ok());
            super::dynamic_textures::rebuild(self);
        }
        workflow.thumbnail_icon = self.studio_library_page.as_ref().and_then(|page| {
            let index = self.textures.dynamic_start() + UI_STUDIO_LIBRARY_PAGE_OFFSET;
            let resident = self.textures.pages().get(index)?;
            if resident.identity() != page.identity() || resident.dimensions() != page.dimensions()
            {
                return None;
            }
            let [width, height] = page.dimensions();
            Some(ui::IconRef {
                page: u16::try_from(index).ok()?,
                uv: [
                    0,
                    0,
                    u16::try_from(width).ok()?,
                    u16::try_from(height).ok()?,
                ],
                glint: false,
            })
        });
    }
}
