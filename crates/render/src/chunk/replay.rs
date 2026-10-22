//! Native replay admission uses the production arena's exact generation and material identities.

use super::*;

/// No incomplete terrain allocation is published as a completed replay frame.
pub(crate) fn ready(world: &mut World) -> bool {
    let mut query = world.query::<(Entity, &render_scene::ChunkRenderInstance)>();
    let Some(source) = world.get_resource::<render_scene::ChunkBiomeTints>() else {
        return false;
    };
    let Some(arena) = world.get_resource::<ChunkGpuArena>() else {
        return false;
    };
    let Some(textures) = world.get_resource::<ChunkGpuTextureAssets>() else {
        return false;
    };
    let Some(active_textures) = world.get_resource::<render_scene::ChunkTextureAssets>() else {
        return false;
    };
    if textures
        .prepared
        .as_ref()
        .is_none_or(|prepared| prepared.identity != active_textures.identity())
        || arena.migration.is_some()
    {
        return false;
    }
    query.iter(world).all(|(entity, instance)| {
        instance.tint_identity != source.identity
            || (instance.cube_quads.is_empty()
                && instance.model_refs.is_empty()
                && instance.liquid_quads.is_empty())
            || arena.allocations.get(&entity).is_some_and(|allocation| {
                allocation.generation == instance.generation
                    && allocation.tint_identity == instance.tint_identity
            })
    })
}
