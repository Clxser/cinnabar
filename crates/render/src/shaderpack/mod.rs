//! Experimental "Shader pack" render mode: runs a Java OptiFine/Iris shader pack, translated
//! by the `shaderpack` crate, on Cinnabar's world. Compiled with the `shaderpacks` feature;
//! without it [`ShaderPackRenderPlugin`] does nothing.
//!
//! Flow: the app writes `ShaderPackRequest`; a worker thread translates the pack and builds
//! raw wgpu pipelines from the naga modules (see `pipelines`, `draws`), publishing
//! `ShaderPackStatus` and the loaded pack. Cameras marked `ShaderPackRendering` then run the
//! frame as Iris does (see `passes`): the `shadow*` programs over resident terrain, actors
//! and block entities into the shadow maps, `shadowcomp*` compute passes, `prepare*`,
//! `gbuffers_skybasic` over the sky discs, opaque terrain with the `gbuffers_terrain*`
//! programs, actors (`gbuffers_entities`, `spidereyes`, `armor_glint`), block entities
//! (`gbuffers_block`) and the hand (`gbuffers_hand`), `deferred*`, translucent terrain,
//! actors, block entities and hand, `composite*` and `final`, before tonemapping and the UI.
//! Terrain comes from the geometry adapter (`geometry`) and the block atlas (`atlas`);
//! actors, block entities and the hand from the actor adapter (`actors`). What the pack does
//! not draw yet (particles, clouds, weather) is taken from the Vanilla scene, which still
//! renders underneath, where it is nearer than the pack's geometry. Packs without terrain
//! programs get the Vanilla scene copied into `colortex0` instead.
//!
//! Conventions: pack buffers are stored bottom-up and gamma-encoded as in OpenGL Minecraft
//! (the translator flips clip-space Y and remaps clip depth), uniforms use OpenGL matrices,
//! and each texture is read with the sampler class Iris gives its name.

#[cfg(feature = "shaderpacks")]
pub(crate) mod actors;
#[cfg(feature = "shaderpacks")]
mod alpha_test;
#[cfg(feature = "shaderpacks")]
pub(crate) mod atlas;
#[cfg(feature = "shaderpacks")]
mod draws;
#[cfg(feature = "shaderpacks")]
mod frame_geometry;
#[cfg(feature = "shaderpacks")]
pub(crate) mod geometry;
#[cfg(feature = "shaderpacks")]
mod host_pipelines;
#[cfg(feature = "shaderpacks")]
mod lightmap;
#[cfg(feature = "shaderpacks")]
mod loader;
#[cfg(feature = "shaderpacks")]
mod passes;
#[cfg(feature = "shaderpacks")]
mod pipelines;
#[cfg(feature = "shaderpacks")]
mod plan;
#[cfg(feature = "shaderpacks")]
mod plugin;
#[cfg(feature = "shaderpacks")]
mod resources;
#[cfg(feature = "shaderpacks")]
mod shadow_maps;
#[cfg(feature = "shaderpacks")]
mod sky;
#[cfg(feature = "shaderpacks")]
mod targets;
#[cfg(all(test, feature = "shaderpacks"))]
mod tests;
#[cfg(feature = "shaderpacks")]
mod uniform_audit;
#[cfg(feature = "shaderpacks")]
mod uniforms;

use bevy::prelude::{App, Plugin};

/// Loads the requested shader pack and runs it on opted-in cameras.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShaderPackRenderPlugin;

impl Plugin for ShaderPackRenderPlugin {
    #[cfg(feature = "shaderpacks")]
    fn build(&self, app: &mut App) {
        plugin::build(app);
    }

    #[cfg(not(feature = "shaderpacks"))]
    fn build(&self, _app: &mut App) {}

    #[cfg(feature = "shaderpacks")]
    fn finish(&self, app: &mut App) {
        plugin::finish(app);
    }
}
