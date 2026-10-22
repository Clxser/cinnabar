//! Raster-only appearance admission for independently validated prepared replay draws.

use super::*;

impl ActorArtworkPages {
    /// Restores raster storage for prepared draws. This does not grant live rig or pack-routing admission.
    pub fn from_recorded_rasters(
        identity: [u8; 32],
        entity_identity: [u8; 32],
        pages: Arc<[ActorTexturePage]>,
        actor_glint: Option<EquipmentRaster>,
        item_glint: Option<EquipmentRaster>,
    ) -> Option<Self> {
        let mut bytes = player_page_bytes();
        for page in pages.iter() {
            let expected = usize::from(page.width)
                .checked_mul(usize::from(page.height))?
                .checked_mul(page.layers as usize)?
                .checked_mul(4)?;
            if page.width == 0
                || page.height == 0
                || page.layers == 0
                || page.layers as usize > MAX_ACTOR_PAGE_LAYERS
                || expected != page.rgba8.len()
            {
                return None;
            }
            bytes = bytes.checked_add(expected)?;
        }
        if !within_page_budget(pages.len(), bytes) {
            return None;
        }
        for raster in [actor_glint.as_ref(), item_glint.as_ref()]
            .into_iter()
            .flatten()
        {
            let expected = usize::from(raster.width)
                .checked_mul(usize::from(raster.height))?
                .checked_mul(4)?;
            if raster.width == 0 || raster.height == 0 || expected != raster.rgba8.len() {
                return None;
            }
            if expected > MAX_ACTOR_GPU_PIXEL_BYTES {
                return None;
            }
        }
        Some(Self {
            identity,
            entity_identity,
            pages,
            actor_glint,
            item_glint,
            ..Self::default()
        })
    }
}
