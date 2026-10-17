//! Per-material block attributes for `mc_Entity` and `at_midBlock.w`, and the seam through
//! which the Java block mapping supplies `block.properties` ids.

use std::sync::Arc;

use assets::{
    BLOCK_VISUAL_VARIANT_COVERED_GRASS, BLOCK_VISUAL_VARIANT_MATERIAL_MASK,
    BLOCK_VISUAL_VARIANT_NONSEASONAL_LEAF, BLOCK_VISUAL_VARIANT_SEASONAL_LEAF, BlockFace,
    NetworkIdMode, RuntimeAssets, SEASONAL_LEAF_MATERIAL_COUNT,
};
use bevy::prelude::Resource;

/// The Java `block.properties` id of a Bedrock block state, by sequential runtime id; -1 when
/// the pack assigns none.
pub type BlockEntityIdFn = Arc<dyn Fn(u32) -> i32 + Send + Sync>;

/// The single seam through which the Java↔Bedrock block mapping supplies `mc_Entity.x`.
/// Insert it into the render world; replacing it with a new `revision` re-expands resident
/// terrain. Without it every block reads -1, Iris's value for unassigned blocks.
#[derive(Resource, Clone)]
pub struct ShaderpackBlockIds {
    pub block_entity_id: BlockEntityIdFn,
    /// Changes whenever `block_entity_id` would answer differently.
    pub revision: u64,
}

/// Iris's `mc_Entity.x` for blocks without a `block.properties` id.
pub const UNASSIGNED_BLOCK_ID: i32 = -1;

/// Two words per material: its block's `block.properties` id (as `i32` bits) and light
/// emission 0 to 15. Packed geometry carries materials, not block states, so each material
/// takes the first block state (in runtime-id order) whose faces, model or world-only
/// variant materials (leaves' world faces, snow-covered grass sides) use it, together with
/// that state's positional variants; a material shared by blocks with different ids keeps the
/// first.
pub(crate) fn material_attribute_words(
    assets: &RuntimeAssets,
    block_entity_id: Option<&(dyn Fn(u32) -> i32 + Send + Sync)>,
) -> Vec<u32> {
    let materials = assets.materials();
    let mut assigned = vec![false; materials.len()];
    let mut words = vec![0_u32; materials.len() * 2];
    for word in words.iter_mut().step_by(2) {
        *word = UNASSIGNED_BLOCK_ID as u32;
    }
    let mut assign = |material: u32, id: i32, emission: u8| {
        let Some(base) = materials.get(material as usize) else {
            return;
        };
        let variants =
            base.variation_start..base.variation_start.saturating_add(base.variation_count);
        for material in std::iter::once(material).chain(variants) {
            let index = material as usize;
            if index < assigned.len() && !assigned[index] {
                assigned[index] = true;
                words[index * 2] = id as u32;
                words[index * 2 + 1] = u32::from(emission);
            }
        }
    };
    let templates = assets.model_templates();
    let quads = assets.model_quads();
    for state in 0..assets.visual_count() as u32 {
        let Some(block) = assets.resolve_known(NetworkIdMode::Sequential, state) else {
            continue;
        };
        let id = block_entity_id.map_or(UNASSIGNED_BLOCK_ID, |lookup| lookup(state));
        let emission = block.light_properties().emission();
        for face in BlockFace::ALL {
            assign(block.face(face).material_id(), id, emission);
        }
        // World-only materials the chunk renderer selects instead of the carried faces.
        let variant = block.variant();
        let base = variant & BLOCK_VISUAL_VARIANT_MATERIAL_MASK;
        let leaf = BLOCK_VISUAL_VARIANT_SEASONAL_LEAF | BLOCK_VISUAL_VARIANT_NONSEASONAL_LEAF;
        if variant & leaf != 0 {
            for material in base..base.saturating_add(SEASONAL_LEAF_MATERIAL_COUNT) {
                assign(material, id, emission);
            }
        } else if variant & BLOCK_VISUAL_VARIANT_COVERED_GRASS != 0 {
            assign(base, id, emission);
        }
        let parts = block
            .model_template()
            .and_then(|first| assets::model_template_parts(templates, first))
            .unwrap_or(&[]);
        for template in parts {
            let start = template.quad_start as usize;
            let end = start.saturating_add(template.quad_count as usize);
            for quad in quads.get(start..end).unwrap_or(&[]) {
                assign(quad.material, id, emission);
            }
        }
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The chunk renderer draws leaves with their world-only materials, not the carried faces,
    /// so those must carry the leaves' `block.properties` id (BSL's waving and subsurface).
    #[test]
    fn leaves_world_materials_carry_the_leaves_block_id() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = root.join(assets::carriers::startup::DEFAULT_ASSET_PATH);
        if !path.is_file() {
            eprintln!(
                "skipping: missing pinned runtime fixture {}",
                path.display()
            );
            return;
        }
        let loaded = assets::carriers::startup::load_runtime_assets(
            assets::carriers::startup::AssetSelection {
                path,
                source: assets::carriers::startup::AssetPathSource::CommandLine,
            },
        )
        .unwrap();
        let assets = &loaded.runtime;
        let mapping = assets::facts::JavaBlockMapping::pinned().unwrap();
        let maps = shaderpack::IdMaps::parse(
            Some("block.10500=oak_leaves\n"),
            None,
            None,
            &Default::default(),
        );
        let table = maps.blocks.bedrock_block_ids(mapping);
        let lookup = move |state: u32| table.get(state as usize).copied().unwrap_or(-1);
        let words = material_attribute_words(assets, Some(&lookup));
        let mut leaves = 0;
        for state in 0..assets.visual_count() as u32 {
            let Some(block) = assets.resolve_known(NetworkIdMode::Sequential, state) else {
                continue;
            };
            let variant = block.variant();
            let leaf = BLOCK_VISUAL_VARIANT_SEASONAL_LEAF | BLOCK_VISUAL_VARIANT_NONSEASONAL_LEAF;
            if lookup(state) != 10500 || variant & leaf == 0 {
                continue;
            }
            let base = variant & BLOCK_VISUAL_VARIANT_MATERIAL_MASK;
            for material in base..base + SEASONAL_LEAF_MATERIAL_COUNT {
                assert_eq!(
                    words[material as usize * 2] as i32,
                    10500,
                    "material {material}"
                );
            }
            leaves += 1;
        }
        assert!(
            leaves > 0,
            "no oak leaves state uses seasonal world materials"
        );
    }
}
