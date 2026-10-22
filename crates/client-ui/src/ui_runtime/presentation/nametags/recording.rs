//! Recording keeps eligible labels before the live camera's frustum rejection.

use super::*;
use crate::ui_runtime::presentation::UiPresentationRuntime;

/// A capture-cadence spare buffer shares ordinary eligibility and cached text extraction.
#[derive(Default)]
pub(super) struct Projection {
    requested: bool,
    anchors: Vec<NametagAnchor>,
}

impl Projection {
    /// Decorated labels are retained before view rejection, within the existing preparation budget.
    pub(super) fn retain(&mut self, anchor: &NametagAnchor) {
        if self.requested && self.anchors.len() < MAX_PRESENTED_NAMETAGS {
            self.anchors.push(anchor.clone());
        }
    }
}

impl NametagTextCache {
    /// Requests one recording projection; ordinary frames allocate no recording buffer.
    pub fn request_recording(&mut self, requested: bool) {
        self.recording.requested = requested;
        self.recording.anchors.clear();
    }

    /// Applies settings shared with the ordinary projection after actor selection.
    pub fn filter_recorded(&mut self, include: impl FnMut(&NametagAnchor) -> bool) {
        self.recording.anchors.retain(include);
    }

    /// Own-tag selection uses the same decoration and frustum callback as remote tags.
    pub fn record_own_tag(&mut self, anchor: &NametagAnchor) {
        self.recording.retain(anchor);
    }

    /// Publishes only requested frames and recycles selection storage for the next capture tick.
    pub fn publish_recording(&mut self, presentation: &mut UiPresentationRuntime, now_millis: u64) {
        if self.recording.requested {
            presentation.publish_recorded_nametags(&self.recording.anchors, now_millis);
            self.recording.anchors.clear();
            self.recording.requested = false;
        }
    }
}

/// Recording owns its atlas so preparing off-screen text cannot evict live font cells.
#[derive(Default)]
pub(in crate::ui_runtime::presentation) struct Recording {
    enabled: bool,
    requested: bool,
    atlas: NametagAtlas,
    scene: Option<Arc<NametagScene>>,
}

impl Recording {
    /// A replaced session font invalidates both cached cells and the last published recording frame.
    pub(in crate::ui_runtime::presentation) fn reset_artwork(&mut self) {
        self.atlas.reset();
        self.scene = None;
        self.requested |= self.enabled;
    }
}

impl UiPresentationRuntime {
    /// Schedules extra label preparation at capture cadence and releases it when capture ends.
    pub fn request_nametag_recording(&mut self, enabled: bool, due: bool) {
        if !enabled {
            self.nametag_recording = Recording::default();
            return;
        }
        self.nametag_recording.enabled = true;
        self.nametag_recording.requested |= due;
    }

    /// A pending request is consumed by the next UI preparation publication.
    pub fn nametag_recording_requested(&self) -> bool {
        self.nametag_recording.enabled && self.nametag_recording.requested
    }

    /// The last capture-cadence frame contains all admitted eligible labels, including off-screen ones.
    pub fn recorded_nametags(&self) -> Option<&Arc<NametagScene>> {
        self.nametag_recording.scene.as_ref()
    }

    /// Publishes font artwork from the same session/font/style owners as the live label scene.
    fn publish_recorded_nametags(&mut self, anchors: &[NametagAnchor], now_millis: u64) {
        if !self.nametag_recording_requested() {
            return;
        }
        let palette = self.formatting_palette().copied().unwrap_or_default();
        self.nametag_recording.atlas.set_palette(palette);
        self.nametag_recording.atlas.set_animation_seed(now_millis);
        let (font, glyphs) = (&self.font, &self.session_glyphs);
        let dynamic_start = self.textures.dynamic_start();
        let style = self
            .screens
            .mod_ui
            .mod_actor_style()
            .and_then(|style| style.nametags.as_ref());
        let mut scene = build_nametag_scene(
            anchors,
            style,
            font,
            &mut self.layouts,
            &mut self.nametag_recording.atlas,
            &|page| {
                super::super::nametag_atlas::font_page(font, page)
                    .or_else(|| glyphs.page(dynamic_start, page))
            },
        );
        if let Some(style) = style {
            super::style::style_scene(&mut scene, style, self.nametag_local_actor);
        }
        self.nametag_recording.scene = Some(Arc::new(scene));
        self.nametag_recording.requested = false;
    }
}

#[cfg(test)]
mod tests;
