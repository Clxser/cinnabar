//! Physical capacity of the shared dropped-item texture array.

use super::{DroppedItemModel, MAX_ITEM_LAYERS, MIN_ITEM_ATLAS_SIDE};
use assets::texture::policy::{MAX_SUBSCRIBER_BYTES, TexturePolicy};

/// Counts layers, including the white layer, that fit at this extent on this device.
#[must_use]
pub fn atlas_layer_limit(side: u32, policy: TexturePolicy, max_array_layers: usize) -> usize {
    policy.page_bytes(side, side).map_or(0, |bytes| {
        (MAX_SUBSCRIBER_BYTES / bytes)
            .min(MAX_ITEM_LAYERS)
            .min(max_array_layers)
    })
}

impl DroppedItemModel {
    /// Counts artwork layers without the reserved white layer.
    #[must_use]
    pub fn texture_layers(&self) -> usize {
        match self {
            Self::Vacant => 0,
            Self::Sprite(_) | Self::NativeSprite(_) => 1,
            Self::Cube(cube) => cube.faces.len(),
            Self::Block(block) => block.materials.len(),
        }
    }

    /// Returns the minimum extent if this model and the white layer fit the physical budget.
    #[must_use]
    pub fn atlas_side(&self, policy: TexturePolicy) -> Option<u32> {
        let side = match self {
            Self::Block(block) => block
                .materials
                .iter()
                .map(|(sprite, _)| sprite.width.max(sprite.height))
                .max(),
            Self::Cube(cube) => Some(cube.tile),
            Self::Sprite(sprite) | Self::NativeSprite(sprite) => {
                Some(sprite.width.max(sprite.height))
            }
            Self::Vacant => None,
        }?
        .checked_next_power_of_two()?
        .max(MIN_ITEM_ATLAS_SIDE);
        let layers = self.texture_layers().checked_add(1)?;
        (layers <= atlas_layer_limit(side, policy, MAX_ITEM_LAYERS)).then_some(side)
    }
}
