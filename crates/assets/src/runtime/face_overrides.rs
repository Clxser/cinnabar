//! Face rebinding for plain full cubes with independent carried cube art.

use crate::{AssetError, BlockFlags, BlockVisual, Material, VisualKind};

/// Replaces named cube faces without moving models or their material groups.
pub(super) fn apply(
    replacements: &[(u32, [Option<u32>; 6])],
    visuals: &mut [BlockVisual],
    materials: &[Material],
    material_range: std::ops::Range<u32>,
) -> Result<(), AssetError> {
    let local_materials = material_range.end - material_range.start;
    let mut seen = std::collections::HashSet::new();
    for &(id, replacement) in replacements {
        if !seen.insert(id)
            || replacement
                .iter()
                .flatten()
                .any(|&id| id >= local_materials)
        {
            return Err(invalid(
                "base face override is duplicated or references an invalid material",
            ));
        }
        let visual = visuals
            .get_mut(id as usize)
            .ok_or_else(|| invalid("base face override visual is out of bounds"))?;
        if visual.kind != VisualKind::Cube
            || visual.model_template != crate::NO_MODEL_TEMPLATE
            || visual.variant != 0
            || visual.flags.contains(BlockFlags::LEAF_MODEL)
            || visual
                .faces
                .iter()
                .any(|&id| materials[id as usize].flags & crate::MATERIAL_FLAG_TINT_MASK != 0)
        {
            continue;
        }
        for (face, material) in replacement.iter().enumerate() {
            if let Some(material) = material {
                visual.faces[face] = material_range.start + material;
            }
        }
    }
    Ok(())
}

/// Names rejected bindings without exposing unchecked runtime references.
fn invalid(detail: &str) -> AssetError {
    AssetError::InvalidCompiledAssets {
        detail: detail.into(),
    }
}
