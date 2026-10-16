//! Current models take priority over speculative frames in the physical GPU array.

use super::*;
use assets::texture::policy::TexturePolicy;
use render_model::{MIN_ITEM_ATLAS_SIDE, dropped_item::atlas_layer_limit};

impl ModelCache {
    /// Keeps current consumers and selects only speculative frames that fit alongside them.
    pub(super) fn publication_mask(
        &self,
        policy: TexturePolicy,
        max_array_layers: usize,
    ) -> [bool; MAX_ITEM_LAYERS - 1] {
        let mut published = [false; MAX_ITEM_LAYERS - 1];
        let mut side = MIN_ITEM_ATLAS_SIDE;
        let mut layers = 1;
        let mut current_fits = true;
        for (slot, model) in self.models.iter().enumerate() {
            if self.demanded(slot) {
                published[slot] = true;
                if let Some(extent) = model.atlas_side(policy) {
                    side = side.max(extent);
                    layers += model.texture_layers();
                } else {
                    current_fits = false;
                }
            }
        }
        // If current demand alone exceeds capacity, preserve its existing renderer admission.
        if !current_fits || layers > atlas_layer_limit(side, policy, max_array_layers) {
            return published;
        }
        for (slot, model) in self.models.iter().enumerate() {
            if published[slot]
                || !self.residents[slot]
                    .as_ref()
                    .is_some_and(|entry| entry.warmed == self.frame)
            {
                continue;
            }
            if let Some(extent) = model.atlas_side(policy) {
                let candidate = side.max(extent);
                let needed = layers + model.texture_layers();
                if needed <= atlas_layer_limit(candidate, policy, max_array_layers) {
                    published[slot] = true;
                    side = candidate;
                    layers = needed;
                }
            }
        }
        published
    }
}
