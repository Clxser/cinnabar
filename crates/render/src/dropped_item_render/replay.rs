//! Independent replay admission never carries terrain interlock progress across backward seeks.

use super::*;

/// Reset the visibility ledger before restoring an older source sample.
pub(crate) fn reset(world: &mut World) {
    if let Some(mut ledger) = world.get_resource_mut::<TerrainItemMeshGenerations>() {
        *ledger = TerrainItemMeshGenerations::default();
    }
}

/// Waits for the selected native item epoch and reports GPU capacity omissions explicitly.
pub(crate) fn ready(world: &World) -> Result<bool, String> {
    let Some(scene) = world.get_resource::<DroppedItemScene>() else {
        return Ok(true);
    };
    let Some(gpu) = world.get_resource::<ItemGpu>() else {
        return Ok(false);
    };
    if scene.instances.is_empty() && scene.terrain_instances.is_empty() && scene.dynamic.is_empty()
    {
        return Ok(true);
    }
    if gpu.models_revision != scene.models_revision
        || gpu.ranges.len() != scene.models.len()
        || gpu.bind_group.is_none()
    {
        return Ok(false);
    }
    let current = current_models(scene);
    if current
        .iter()
        .enumerate()
        .any(|(slot, current)| *current && gpu.ranges[slot].capacity_rejected)
    {
        return Err("Recorded item models exceed the native replay GPU geometry budget.".into());
    }
    Ok(true)
}
