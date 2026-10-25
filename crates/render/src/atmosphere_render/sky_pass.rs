//! The ordered sky pass fills the colour samples no opaque world geometry covers.
//!
//! The sky never writes depth and every layer tests against far depth, so drawing it after opaque
//! geometry leaves it exactly the samples that geometry left uncovered. That order is only taken
//! when each opaque draw replaces the colour of exactly the samples it gives depth; otherwise the
//! sky initializes colour before world geometry, as the uncovered samples then also depend on it.

use super::{AtmosphereGpu, AtmospherePipelineKey};
use bevy::prelude::IntoScheduleConfigs;
use bevy::{
    camera::{MainPassResolutionOverride, Viewport},
    core_pipeline::{
        Core3d, Core3dSystems,
        core_3d::{AlphaMask3d, Opaque3d},
    },
    ecs::query::Has,
    prelude::{Entity, Msaa, Resource, World},
    render::{
        camera::ExtractedCamera,
        render_phase::ViewBinnedRenderPhases,
        render_resource::{
            CachedPipelineState, CachedRenderPipelineId, ColorWrites, Pipeline, PipelineCache,
            RenderPassDescriptor, RenderPipelineDescriptor, StoreOp,
        },
        renderer::{RenderContext, ViewQuery},
        view::{ExtractedView, ViewDepthStencilTexture, ViewTarget, ViewUniformOffset},
    },
};

#[derive(Resource)]
struct SkyPassInstalled;

/// Places the sky either before world geometry or after every opaque world pass, the latter
/// before entity shadows and transparency that blend over it. Each view runs exactly one slot.
/// Bevy's stock main passes bound the later slot in apps without the shared scene target.
pub(super) fn install(world: &mut World) {
    if world.contains_resource::<SkyPassInstalled>() {
        return;
    }
    if world
        .try_schedule_scope(Core3d, |_, schedule| {
            schedule.add_systems((
                crate::gpu_timing::profiled(
                    atmosphere::<false>,
                    Some(render_scene::RuntimeStage::GpuSky),
                    "AtmospherePass",
                )
                .after(Core3dSystems::Prepass)
                .before(Core3dSystems::MainPass)
                .run_if(slot::<false>),
                crate::gpu_timing::profiled(
                    atmosphere::<true>,
                    Some(render_scene::RuntimeStage::GpuSky),
                    "AtmosphereAfterOpaquePass",
                )
                .after(crate::scene_target::ScenePass::Opaque)
                .after(bevy::core_pipeline::core_3d::main_opaque_pass_3d)
                .after(crate::chunk::GpuCullLateLabel)
                .after(crate::far_terrain::FarTerrainLabel)
                .before(crate::entity_shadow_render::EntityShadowLabel)
                .before(crate::scene_target::ScenePass::Transparent)
                .before(bevy::core_pipeline::core_3d::main_transparent_pass_3d)
                .in_set(Core3dSystems::MainPass)
                .run_if(slot::<true>),
            ));
        })
        .is_ok()
    {
        world.insert_resource(SkyPassInstalled);
    }
}

type SlotQuery = (
    &'static ExtractedView,
    &'static ExtractedCamera,
    &'static Msaa,
    Has<render_scene::EnhancedRendering>,
);

/// Selects the view's sky slot; both slots evaluate the same frame state.
fn slot<const AFTER_OPAQUE: bool>(world: &World, view: ViewQuery<SlotQuery>) -> bool {
    let entity = view.entity();
    let (view, camera, msaa, enhanced) = view.into_inner();
    after_opaque(world, entity, view, *msaa, camera.hdr, enhanced) == AFTER_OPAQUE
}

/// Whether the sky can follow this view's opaque geometry without changing any sample: every
/// pipeline the opaque pass binds must cover the sky. Pipelines that are not ready draw nothing.
/// The GPU-cull late pass and far terrain reuse terrain-style pipelines that always cover it.
pub(crate) fn after_opaque(
    world: &World,
    entity: Entity,
    view: &ExtractedView,
    msaa: Msaa,
    hdr: bool,
    enhanced: bool,
) -> bool {
    // Bevy's own skybox draws at far depth inside the opaque pass.
    if world
        .get::<bevy::core_pipeline::skybox::SkyboxPipelineId>(entity)
        .is_some()
    {
        return false;
    }
    let phases = (
        world.get_resource::<ViewBinnedRenderPhases<Opaque3d>>(),
        world.get_resource::<ViewBinnedRenderPhases<AlphaMask3d>>(),
    );
    let (Some(opaque), Some(cutout)) = (
        phases
            .0
            .and_then(|phases| phases.get(&view.retained_view_entity)),
        phases
            .1
            .and_then(|phases| phases.get(&view.retained_view_entity)),
    ) else {
        // The opaque pass draws nothing without both phases.
        return true;
    };
    let cache = world.resource::<PipelineCache>();
    cutout.is_empty()
        && opaque.multidrawable_meshes.is_empty()
        && opaque.batchable_meshes.is_empty()
        && opaque.unbatchable_meshes.is_empty()
        && opaque
            .non_mesh_items
            .keys()
            .all(|(batch, _)| pipeline_covers_sky(cache, batch.pipeline))
        && crate::actor_render::phase::opaque_span_pipelines(world, msaa, hdr, enhanced)
            .all(|id| pipeline_covers_sky(cache, id))
}

/// A ready pipeline must cover the sky; one that is not ready yet draws nothing this frame.
fn pipeline_covers_sky(cache: &PipelineCache, id: CachedRenderPipelineId) -> bool {
    match cache.get_render_pipeline_state(id) {
        CachedPipelineState::Ok(Pipeline::RenderPipeline(_)) => {
            covers_sky(cache.get_render_pipeline_descriptor(id))
        }
        CachedPipelineState::Ok(_) => false,
        _ => true,
    }
}

/// Whether a pipeline replaces the colour of exactly the samples it gives depth: it writes
/// depth, and its single colour target writes RGB without blending. Such geometry leaves the
/// same colour whether or not the sky was drawn beneath it, and depth on every sample it touched.
pub(crate) fn covers_sky(descriptor: &RenderPipelineDescriptor) -> bool {
    descriptor
        .depth_stencil
        .as_ref()
        .is_some_and(|depth| depth.depth_write_enabled == Some(true))
        && descriptor.fragment.as_ref().is_some_and(|fragment| {
            matches!(
                fragment.targets.as_slice(),
                [Some(target)] if target.blend.is_none()
                    && target.write_mask.contains(ColorWrites::COLOR)
            )
        })
}

type AtmosphereQuery = (
    &'static ExtractedCamera,
    &'static ViewTarget,
    Option<&'static crate::scene_target::SceneTarget>,
    &'static ViewDepthStencilTexture,
    &'static ViewUniformOffset,
    &'static Msaa,
    Option<&'static MainPassResolutionOverride>,
);

/// Draws the stock sky, stars and finite pack cube against far depth, except under a shader pack
/// that draws the view's sky and world itself.
pub(super) fn atmosphere<const AFTER_OPAQUE: bool>(
    world: &World,
    query: ViewQuery<AtmosphereQuery>,
    mut context: RenderContext,
) {
    #[cfg(feature = "shaderpacks")]
    let view = query.entity();
    if crate::scene_target::unavailable(world, query.entity()) {
        return;
    }
    let (camera, target, scene, depth, offset, msaa, resolution) = query.into_inner();
    if world
        .get_resource::<render_scene::PanoramaScene>()
        .is_some_and(|scene| !scene.game_visible())
    {
        return;
    }
    #[cfg(feature = "shaderpacks")]
    if crate::shaderpack::plugin::draws_world(world, view) {
        return;
    }
    let Some(gpu) = world.get_resource::<AtmosphereGpu>() else {
        return;
    };
    let Some(bind_group) = gpu.bind_group.as_ref() else {
        return;
    };
    let cache = world.resource::<PipelineCache>();
    let key = AtmospherePipelineKey {
        msaa: *msaa,
        hdr: camera.hdr,
        stars: false,
        skybox: false,
    };
    let Some(stock) = gpu
        .pipelines
        .get(&key)
        .and_then(|id| cache.get_render_pipeline(*id))
    else {
        return;
    };
    let colour = scene.map_or_else(
        || target.get_color_attachment(),
        |scene| scene.color_attachment(target, false),
    );
    let extent = crate::render_bounds::extent(colour.view);
    let viewport = Viewport::from_viewport_and_override(camera.viewport.as_ref(), resolution)
        .map(|viewport| crate::render_bounds::viewport(&viewport, extent));
    let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some(if AFTER_OPAQUE {
            "sky after opaque world geometry"
        } else {
            "sky before world geometry"
        }),
        color_attachments: &[Some(colour)],
        depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
        timestamp_writes: crate::gpu_timing::render_pass_timestamps(
            world,
            render_scene::RuntimeStage::GpuSky,
        ),
        occlusion_query_set: None,
        multiview_mask: None,
    });
    match viewport {
        Some(Some(viewport)) => pass.set_camera_viewport(&viewport),
        Some(None) => return,
        None => {}
    }
    pass.set_render_pipeline(stock);
    pass.set_bind_group(0, bind_group, &[offset.offset]);
    pass.draw(0..3, 0..1);
    for (stars, skybox) in [(true, false), (false, true)] {
        if skybox && !gpu.skybox_active {
            continue;
        }
        let key = AtmospherePipelineKey {
            stars,
            skybox,
            ..key
        };
        if let Some(pipeline) = gpu
            .pipelines
            .get(&key)
            .and_then(|id| cache.get_render_pipeline(*id))
        {
            pass.set_render_pipeline(pipeline);
            if stars {
                pass.draw(3..3 + gpu.star_vertex_count, 0..1);
            } else {
                pass.draw(0..3, 0..1);
            }
        }
    }
}
