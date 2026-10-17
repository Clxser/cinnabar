//! Bevy wiring: the main-world load worker, extraction, the geometry adapter's settings and
//! block ids, the actor adapter's id maps and Vanilla hand-off, per-view preparation and the
//! render-graph system.

use super::actors::{self, ActorSkinPipelines, ActorSources, RigSource};
use super::atlas::{ShaderpackAtlas, ShaderpackAtlasPlugin};
use super::draws::DrawKind;
use super::effects::resources::VanillaTextures;
use super::effects::textures::{ShaderpackEffectTextures, prepare_effect_textures};
use super::effects::{Effect, EffectCamera, EffectInputs};
use super::frame_geometry::{FrameGeometry, TerrainDraws};
use super::geometry::{
    GeometryCategory, ShaderpackBlockIds, ShaderpackGeometry, ShaderpackGeometryPlugin,
    ShaderpackGeometrySettings,
};
use super::loader::{GpuContext, PackGpu, load};
use super::passes::{DrawContext, FrameIo, ViewState, record_frame};
use super::sky::SkyInputs;
use super::targets::AtlasViews;
use super::uniforms::{FAR_PLANE_SCALE, FrameClock, FrameUniforms, ViewCamera};
use bevy::render::render_phase::TrackedRenderPass;
use bevy::render::render_resource::PipelineCache;
use bevy::{
    core_pipeline::{Core3d, Core3dSystems},
    prelude::*,
    render::{
        Render, RenderApp, RenderSystems,
        extract_component::ExtractComponentPlugin,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        renderer::{RenderAdapter, RenderContext, RenderDevice, RenderQueue, ViewQuery},
        view::{ExtractedView, ViewDepthStencilTexture, ViewTarget},
    },
};
use render_scene::shaderpack::{
    ShaderPackActorIds, ShaderPackDrawnCategories, ShaderPackEffects, ShaderPackFrameInputs,
    ShaderPackIdMaps, ShaderPackRendering, ShaderPackRequest, ShaderPackStatus,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
};

/// Java's first-person field of view, for the hand projection while no hand rig draws.
const DEFAULT_HAND_FOV_DEGREES: f32 = 70.0;

/// The loaded pack, shared with the render world.
#[derive(Resource, ExtractResource, Clone, Default)]
#[extract_app(bevy::render::RenderApp)]
pub(crate) struct LoadedShaderPack(pub Option<Arc<PackGpu>>);

type LoadResult = Result<PackGpu, String>;

/// The pack being loaded or shown, identified by path and dimension.
#[derive(Resource, Default)]
struct Loader {
    key: Option<(PathBuf, i32)>,
    job: Option<Mutex<mpsc::Receiver<LoadResult>>>,
}

/// The render device, queue and adapter Bevy also publishes in the main world.
type MainWorldGpu<'w> = (
    Option<Res<'w, RenderDevice>>,
    Option<Res<'w, RenderQueue>>,
    Option<Res<'w, RenderAdapter>>,
);

/// Starts a load when the requested pack or the dimension changes and publishes results.
/// A dimension change of the shown pack keeps it on screen until the new folder is ready.
/// The pack's `minecraft:` texture references resolve against the vanilla textures loaded
/// when its load starts.
fn drive_loader(
    request: Res<ShaderPackRequest>,
    inputs: Res<ShaderPackFrameInputs>,
    mut status: ResMut<ShaderPackStatus>,
    mut loaded: ResMut<LoadedShaderPack>,
    mut loader: ResMut<Loader>,
    gpu: MainWorldGpu,
    atmosphere: Option<Res<render_scene::AtmosphereTextureAssets>>,
) {
    let wanted = request
        .pack
        .clone()
        .map(|path| (path, inputs.dimension.clamp(0, 2)));
    if wanted != loader.key {
        let Some((path, dimension)) = wanted.clone() else {
            loader.key = None;
            loader.job = None;
            status.set_if_neq(ShaderPackStatus::Idle);
            loaded.0 = None;
            return;
        };
        let (Some(device), Some(queue), Some(adapter)) = gpu else {
            return;
        };
        loader.key = wanted;
        loader.job = None;
        let gpu = GpuContext {
            device: device.wgpu_device().clone(),
            queue: (**queue).clone(),
            adapter: (**adapter).clone(),
        };
        let reloading = matches!(&*status, ShaderPackStatus::Ready(shown) if *shown == path);
        if !reloading {
            status.set_if_neq(ShaderPackStatus::Loading(path.clone()));
            loaded.0 = None;
        }
        let cache_dir = request.cache_dir.clone();
        let vanilla = atmosphere
            .as_deref()
            .map(VanillaTextures::from_atmosphere)
            .unwrap_or_default();
        let (send, receive) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("shader pack load".into())
            .spawn(move || {
                let id = shaderpack::JAVA_DIMENSIONS[dimension as usize];
                let _ = send.send(load(&path, id, cache_dir.as_deref(), &gpu, &vanilla));
            });
        match spawned {
            Ok(_) => loader.job = Some(Mutex::new(receive)),
            Err(error) => {
                if let Some((path, _)) = &loader.key {
                    *status = ShaderPackStatus::Failed {
                        pack: path.clone(),
                        reason: error.to_string(),
                    };
                }
            }
        }
        return;
    }
    let Some(job) = loader.job.as_mut() else {
        return;
    };
    let received = job.get_mut().unwrap_or_else(|e| e.into_inner()).try_recv();
    let outcome = match received {
        Ok(outcome) => outcome,
        Err(mpsc::TryRecvError::Empty) => return,
        Err(mpsc::TryRecvError::Disconnected) => Err("the load worker stopped".to_string()),
    };
    loader.job = None;
    let Some((path, _)) = loader.key.clone() else {
        return;
    };
    match outcome {
        Ok(pack) => {
            loaded.0 = Some(Arc::new(pack));
            *status = ShaderPackStatus::Ready(path);
        }
        Err(reason) => {
            warn!(
                target: "cinnabar::shaderpack",
                pack = %path.display(),
                "shader pack unavailable, rendering Vanilla: {reason}"
            );
            loaded.0 = None;
            *status = ShaderPackStatus::Failed { pack: path, reason };
        }
    }
}

/// Render-world state of every view drawing through the pack.
#[derive(Resource, Default)]
struct ShaderPackViews(Mutex<HashMap<Entity, ViewState>>);

type PreparedView = (Entity, &'static ExtractedView);

/// Enables the geometry adapter while a pack with terrain programs is shown on some camera,
/// with the pack's `separateAo`/`oldLighting`, publishes its block ids, tells the Vanilla
/// terrain, actor, block-entity and hand passes which of them the pack draws itself, and
/// publishes the effects it draws in place of Vanilla's.
#[allow(clippy::too_many_arguments, reason = "a render-world system's inputs")]
fn configure_geometry(
    mut commands: Commands,
    pack: Option<Res<LoadedShaderPack>>,
    cameras: Query<(), With<ShaderPackRendering>>,
    (mut settings, geometry): (
        ResMut<ShaderpackGeometrySettings>,
        Option<Res<ShaderpackGeometry>>,
    ),
    mut drawn: ResMut<ShaderPackDrawnCategories>,
    ids: Option<Res<ShaderpackBlockIds>>,
    mut replaced: ResMut<ShaderPackEffects>,
    mut shown: Local<Option<usize>>,
) {
    let pack = pack
        .and_then(|p| p.0.clone())
        .filter(|p| !p.draws.is_empty() && !cameras.is_empty());
    let wanted = ShaderpackGeometrySettings {
        enabled: pack.is_some(),
        separate_ao: pack.as_ref().is_some_and(|p| p.separate_ao),
        old_lighting: pack.as_ref().is_some_and(|p| p.old_lighting),
        ..settings.clone()
    };
    if *settings != wanted {
        *settings = wanted;
    }
    replaced.set_if_neq(
        pack.as_deref()
            .map_or_else(Default::default, replaced_effects),
    );
    let draws = |kind: DrawKind| {
        pack.as_ref()
            .is_some_and(|p| p.draws.iter().any(|d| d.kind == kind))
    };
    let expands_terrain = geometry.is_some_and(|geometry| geometry.supported());
    drawn.set_if_neq(ShaderPackDrawnCategories {
        terrain: expands_terrain
            && (draws(DrawKind::TerrainSolid) || draws(DrawKind::TerrainCutout)),
        actors: draws(DrawKind::Entities),
        block_entities: draws(DrawKind::Block),
        hand: draws(DrawKind::Hand),
    });
    let key = pack.as_ref().map(|p| Arc::as_ptr(p) as usize);
    if *shown == key {
        return;
    }
    *shown = key;
    let table = pack.and_then(|p| p.block_ids.clone());
    match table {
        Some(table) => {
            let revision = ids.map_or(1, |ids| ids.revision + 1);
            commands.insert_resource(ShaderpackBlockIds {
                block_entity_id: Arc::new(move |state| {
                    table.get(state as usize).copied().unwrap_or(-1)
                }),
                revision,
            });
        }
        None => commands.remove_resource::<ShaderpackBlockIds>(),
    }
}

/// The Vanilla effects `pack` replaces: those it draws, and clouds and weather its
/// directives turn off.
fn replaced_effects(pack: &PackGpu) -> ShaderPackEffects {
    let draws = |effects: &[Effect]| {
        pack.draws
            .iter()
            .any(|d| matches!(d.kind, DrawKind::Effect(e) if effects.contains(&e)))
    };
    ShaderPackEffects {
        particles: draws(&[
            Effect::Particles,
            Effect::ParticlesTranslucent,
            Effect::ParticlesAdditive,
        ]),
        weather: draws(&[Effect::Weather]) || !pack.effects.draws(Effect::Weather),
        lightning: draws(&[Effect::Lightning]),
        clouds: draws(&[Effect::Clouds]) || !pack.effects.draws(Effect::Clouds),
        selection: draws(&[Effect::Outline]),
        cracks: draws(&[Effect::DamagedBlock]),
    }
}

/// Allocates or resizes view targets and writes this frame's uniforms.
#[allow(clippy::too_many_arguments, reason = "one frame's view inputs")]
fn prepare_views(
    mut views: ResMut<ShaderPackViews>,
    pack: Option<Res<LoadedShaderPack>>,
    inputs: Option<Res<ShaderPackFrameInputs>>,
    cameras: Query<PreparedView, With<ShaderPackRendering>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    time: Option<Res<Time>>,
    geometry: Option<Res<ShaderpackGeometry>>,
    atlas: Option<Res<ShaderpackAtlas>>,
    effect_textures: Option<Res<ShaderpackEffectTextures>>,
    hand: Option<Res<render_scene::HandRigScene>>,
) {
    let hand_fov = hand
        .as_ref()
        .and_then(|hand| hand.frame.as_ref())
        .map_or(DEFAULT_HAND_FOV_DEGREES.to_radians(), |frame| {
            frame.fov_radians
        });
    let states = views.0.get_mut().unwrap_or_else(|e| e.into_inner());
    let (Some(pack), Some(inputs)) = (pack.and_then(|p| p.0.clone()), inputs) else {
        states.clear();
        return;
    };
    states.retain(|entity, _| cameras.contains(*entity));
    let clock = FrameClock {
        delta_seconds: time.map_or(0.0, |t| t.delta_secs()),
    };
    for (entity, view) in &cameras {
        let screen = [view.viewport.z.max(1), view.viewport.w.max(1)];
        let state = states
            .entry(entity)
            .or_insert_with(|| ViewState::new(device.wgpu_device(), Arc::clone(&pack), screen));
        if !state.matches(&pack, screen) {
            *state = ViewState::new(device.wgpu_device(), Arc::clone(&pack), screen);
        }
        let (_, rotation, position) = view.world_from_view.to_scale_rotation_translation();
        let camera = ViewCamera {
            position,
            rotation,
            focal: [view.clip_from_view.x_axis.x, view.clip_from_view.y_axis.y],
            screen,
        };
        let frame = FrameUniforms::new(
            camera,
            &inputs,
            &state.pack.settings,
            clock,
            &mut state.history,
        );
        let chunk_offset = geometry
            .as_ref()
            .and_then(|g| g.chunk_offset(position.as_dvec3()))
            .unwrap_or(Vec3::ZERO);
        let atlas_size = atlas.as_ref().map_or([0; 2], |atlas| {
            atlas.size.to_array().map(|side| side as i32)
        });
        let far = inputs.render_distance.max(16.0) * FAR_PLANE_SCALE;
        let context = DrawContext {
            chunk_offset,
            atlas_size,
            particle_atlas_size: effect_textures
                .as_ref()
                .map_or([0; 2], |t| t.particle_atlas_size()),
            hand_projection: actors::hand_projection(hand_fov, screen, far),
        };
        state.write_uniforms(&queue, &frame, clock.delta_seconds, &context);
    }
}

type PostQuery = (
    &'static ViewTarget,
    &'static ViewDepthStencilTexture,
    &'static ExtractedView,
);

/// The camera view's terrain through the geometry adapter.
struct ViewTerrain<'a> {
    geometry: &'a ShaderpackGeometry,
    view: Entity,
}

impl TerrainDraws for ViewTerrain<'_> {
    fn draw<'w>(
        &'w self,
        pass: &mut TrackedRenderPass<'w>,
        category: GeometryCategory,
        shadow: bool,
    ) -> u32 {
        let view = (!shadow).then_some(self.view);
        self.geometry.draw(pass, category, view)
    }
}

/// Runs the pack over the frame: its shadow pass, geometry and full-screen chain, and
/// presents `final`.
fn shaderpack_post(
    world: &World,
    query: ViewQuery<PostQuery, With<ShaderPackRendering>>,
    mut context: RenderContext,
) -> bevy::ecs::error::Result {
    let entity = query.entity();
    let (target, depth, view) = query.into_inner();
    let Some(views) = world.get_resource::<ShaderPackViews>() else {
        return Ok(());
    };
    let mut states = views.0.lock().unwrap_or_else(|e| e.into_inner());
    let Some(state) = states.get_mut(&entity) else {
        return Ok(());
    };
    let inputs = world
        .get_resource::<ShaderPackFrameInputs>()
        .copied()
        .unwrap_or_default();
    let render_device = world.resource::<RenderDevice>();
    let device = render_device.wgpu_device().clone();
    let queue: wgpu::Queue = (**world.resource::<RenderQueue>()).clone();
    let destination_format = target.main_texture_format();
    let terrain = world
        .get_resource::<ShaderpackGeometry>()
        .filter(|g| g.supported() && g.origin().is_some())
        .map(|geometry| ViewTerrain {
            geometry,
            view: entity,
        });
    let atlas = world.get_resource::<ShaderpackAtlas>().map(|atlas| {
        (
            AtlasViews {
                color: &atlas.color,
                normals: &atlas.normals,
                specular: &atlas.specular,
            },
            atlas.generation,
        )
    });
    let (_, rotation, position) = view.world_from_view.to_scale_rotation_translation();
    let colours = super::uniforms::sky_fog(&inputs, rotation, position);
    let effects = EffectInputs {
        camera: EffectCamera {
            position: view.world_from_view.translation(),
            model_view: Mat4::from_quat(view.world_from_view.rotation().inverse()),
            focal: [view.clip_from_view.x_axis.x, view.clip_from_view.y_axis.y],
            screen: [view.viewport.z.max(1), view.viewport.w.max(1)],
        },
        frame: inputs,
        atmosphere: world.get_resource::<render_scene::AtmosphereFrame>(),
        atmosphere_view: world
            .get_resource::<render_scene::AtmosphereViewInputs>()
            .copied(),
        atmosphere_textures: world.get_resource::<render_scene::AtmosphereTextureAssets>(),
        clouds_visible: world
            .get_resource::<render_scene::CloudVisibility>()
            .is_some_and(|visible| visible.0),
        particles: world.get_resource::<render_scene::ParticleGpuFrame>(),
        precipitation: world.get_resource::<render_scene::PrecipitationScene>(),
        lightning: world.get_resource::<render_scene::LightningScene>(),
        selection: world
            .get_resource::<render_scene::BlockSelectionFrameResource>()
            .map(|frame| &frame.0),
        block_entities: world
            .get_resource::<render_scene::BlockEntityFrameResource>()
            .map(|frame| &frame.0),
    };
    let actor_rig = crate::actor_render::shaderpack_source::shaderpack_actors(world);
    let hand_rig = crate::hand_rig_render::shaderpack_source::shaderpack_hand(world);
    let kernels = world
        .get_resource::<ActorSkinPipelines>()
        .zip(world.get_resource::<PipelineCache>())
        .and_then(|(pipelines, cache)| pipelines.kernels(cache));
    let actors = ActorSources {
        kernels,
        actors: actor_rig.as_ref().map(|rig| rig as &dyn RigSource),
        hand: hand_rig.as_ref().map(|rig| rig as &dyn RigSource),
        block_entities: crate::block_entity::shaderpack_block_entities(world),
        entity_ids: world.get_resource::<ShaderPackActorIds>(),
        held_items: [inputs.held_item_id, inputs.held_item_id2],
        camera: view.world_from_view.translation(),
        seconds: world
            .get_resource::<Time>()
            .map_or(0.0, |time| time.elapsed_secs()),
    };
    let geometry = terrain.as_ref().map(|terrain| FrameGeometry {
        device: render_device,
        terrain,
        atlas,
        effect_textures: world.get_resource::<ShaderpackEffectTextures>(),
        sky: SkyInputs {
            sky_color: colours.sky,
            celestial_angle: inputs.celestial_angle,
            camera_y: position.y,
            sunrise: inputs.dimension == 0,
        },
        effects,
        overlay_radius: terrain.geometry.complete_radius().unwrap_or(f32::MAX),
        actors,
    });
    let post = target.post_process_write();
    let io = FrameIo {
        scene_colour: post.source,
        scene_depth: crate::scene_sampling::view_depth(depth),
        scene_near: view.clip_from_view.w_axis.z,
        inverse_focal: [
            view.clip_from_view.x_axis.x.recip(),
            view.clip_from_view.y_axis.y.recip(),
        ],
        render_distance: inputs.render_distance,
        fog_color: colours.fog,
        destination: post.destination,
        destination_format,
        geometry,
    };
    record_frame(&device, &queue, context.command_encoder(), state, &io);
    Ok(())
}

/// The pack's render-graph system: after the Vanilla scene (whose colour and depth stand
/// in for geometry the pack does not draw) and before tonemapping and the UI.
#[derive(Debug, Hash, PartialEq, Eq, Clone, SystemSet)]
pub(crate) struct ShaderpackPass;

#[derive(Resource)]
struct GraphInstalled;

fn install_graph(world: &mut World) {
    if world.contains_resource::<GraphInstalled>() {
        return;
    }
    let installed = world
        .try_schedule_scope(Core3d, |_, schedule| {
            use bevy::core_pipeline::tonemapping::tonemapping;
            schedule.configure_sets(
                ShaderpackPass
                    .after(bevy::post_process::bloom::bloom)
                    .before(tonemapping)
                    .in_set(Core3dSystems::PostProcess),
            );
            schedule.add_systems(shaderpack_post.in_set(ShaderpackPass));
        })
        .is_ok();
    if installed {
        world.insert_resource(GraphInstalled);
    }
}

/// Publishes the loaded pack's entity and item ids for the app's actor and held-item ids.
fn publish_id_maps(loaded: Res<LoadedShaderPack>, mut maps: ResMut<ShaderPackIdMaps>) {
    if !loaded.is_changed() {
        return;
    }
    let revision = maps.revision + 1;
    *maps = match &loaded.0 {
        Some(pack) => pack.actor_ids.lookups(revision),
        None => {
            let mut empty = ShaderPackIdMaps::default();
            empty.revision = revision;
            empty
        }
    };
}

pub(super) fn build(app: &mut App) {
    actors::load_shaders(app);
    app.init_resource::<ShaderPackRequest>()
        .init_resource::<ShaderPackStatus>()
        .init_resource::<ShaderPackFrameInputs>()
        .init_resource::<ShaderPackIdMaps>()
        .init_resource::<ShaderPackActorIds>()
        .init_resource::<LoadedShaderPack>()
        .init_resource::<Loader>()
        .add_plugins((
            ExtractComponentPlugin::<ShaderPackRendering>::default(),
            ExtractResourcePlugin::<LoadedShaderPack>::default(),
            ExtractResourcePlugin::<ShaderPackFrameInputs>::default(),
            ExtractResourcePlugin::<ShaderPackActorIds>::default(),
            ShaderpackGeometryPlugin,
            ShaderpackAtlasPlugin,
        ))
        .add_systems(Update, (drive_loader, publish_id_maps).chain());
}

pub(super) fn finish(app: &mut App) {
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_app
        .init_resource::<ShaderPackViews>()
        .init_resource::<ShaderPackEffects>()
        .init_resource::<ShaderpackEffectTextures>()
        .init_resource::<ShaderPackDrawnCategories>()
        .init_resource::<ActorSkinPipelines>()
        .add_systems(
            Render,
            (
                configure_geometry
                    .in_set(RenderSystems::PrepareResources)
                    .before(super::geometry::prepare_shaderpack_geometry),
                prepare_effect_textures
                    .in_set(RenderSystems::PrepareResources)
                    .before(prepare_views),
                prepare_views
                    .in_set(RenderSystems::PrepareResources)
                    .after(bevy::render::view::prepare_view_targets)
                    .after(super::geometry::prepare_shaderpack_geometry),
                super::lightmap::update_lightmap.in_set(RenderSystems::PrepareResources),
            ),
        );
    install_graph(render_app.world_mut());
}
