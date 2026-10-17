//! Bevy wiring: the main-world load worker, extraction, per-view preparation and the
//! render-graph systems.

use super::loader::{GpuContext, PackGpu, load};
use super::passes::{FrameIo, ViewState, record_frame};
use super::uniforms::{FrameClock, FrameUniforms, ViewCamera};
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
    ShaderPackFrameInputs, ShaderPackRendering, ShaderPackRequest, ShaderPackStatus,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
};

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
fn drive_loader(
    request: Res<ShaderPackRequest>,
    inputs: Res<ShaderPackFrameInputs>,
    mut status: ResMut<ShaderPackStatus>,
    mut loaded: ResMut<LoadedShaderPack>,
    mut loader: ResMut<Loader>,
    gpu: MainWorldGpu,
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
        let (send, receive) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("shader pack load".into())
            .spawn(move || {
                let id = shaderpack::JAVA_DIMENSIONS[dimension as usize];
                let _ = send.send(load(&path, id, cache_dir.as_deref(), &gpu));
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

/// Allocates or resizes view targets and writes this frame's uniforms.
fn prepare_views(
    mut views: ResMut<ShaderPackViews>,
    pack: Option<Res<LoadedShaderPack>>,
    inputs: Option<Res<ShaderPackFrameInputs>>,
    cameras: Query<PreparedView, With<ShaderPackRendering>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    time: Option<Res<Time>>,
) {
    let states = views.0.get_mut().unwrap_or_else(|e| e.into_inner());
    let (Some(pack), Some(inputs)) = (pack.and_then(|p| p.0.clone()), inputs) else {
        states.clear();
        return;
    };
    states.retain(|entity, _| cameras.contains(*entity));
    let clock = time.map_or(
        FrameClock {
            elapsed_seconds: 0.0,
            delta_seconds: 0.0,
        },
        |t| FrameClock {
            elapsed_seconds: t.elapsed_secs_f64(),
            delta_seconds: t.delta_secs(),
        },
    );
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
        state.write_uniforms(&queue, &frame, clock.delta_seconds);
    }
}

type PostQuery = (
    &'static ViewTarget,
    &'static ViewDepthStencilTexture,
    &'static ExtractedView,
);

/// Runs the pack's full-screen chain over the finished scene and presents `final`.
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
    let render_distance = world
        .get_resource::<ShaderPackFrameInputs>()
        .map_or(ShaderPackFrameInputs::default().render_distance, |i| {
            i.render_distance
        });
    let device = world.resource::<RenderDevice>().wgpu_device().clone();
    let queue: wgpu::Queue = (**world.resource::<RenderQueue>()).clone();
    let destination_format = target.main_texture_format();
    let post = target.post_process_write();
    let io = FrameIo {
        scene_colour: post.source,
        scene_depth: crate::scene_sampling::view_depth(depth),
        scene_near: view.clip_from_view.w_axis.z,
        render_distance,
        destination: post.destination,
        destination_format,
    };
    record_frame(&device, &queue, context.command_encoder(), state, &io);
    Ok(())
}

/// Integration point for the pack's geometry programs (`gbuffers_*`, `shadow`): it will
/// receive the view's [`ViewTargets`](super::targets::ViewTargets) and record the geometry
/// adapter's draws into the colour buffers and depth, replacing the Vanilla scene copy. Until
/// that adapter lands it records nothing and the Vanilla scene stands in for gbuffers.
fn shaderpack_gbuffers() {}

/// The pack's render-graph systems.
#[derive(Debug, Hash, PartialEq, Eq, Clone, SystemSet)]
pub(crate) enum ShaderpackPass {
    /// Geometry programs (currently none run).
    ShaderpackGbuffers,
    /// The full-screen chain and presentation.
    ShaderpackPost,
}

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
                ShaderpackPass::ShaderpackGbuffers
                    .after(crate::scene_target::ScenePass::Transparent)
                    .before(crate::scene_target::ScenePass::Finish)
                    .in_set(Core3dSystems::MainPass),
            );
            schedule.configure_sets(
                ShaderpackPass::ShaderpackPost
                    .after(bevy::post_process::bloom::bloom)
                    .before(tonemapping)
                    .in_set(Core3dSystems::PostProcess),
            );
            schedule.add_systems((
                shaderpack_gbuffers.in_set(ShaderpackPass::ShaderpackGbuffers),
                shaderpack_post.in_set(ShaderpackPass::ShaderpackPost),
            ));
        })
        .is_ok();
    if installed {
        world.insert_resource(GraphInstalled);
    }
}

pub(super) fn build(app: &mut App) {
    app.init_resource::<ShaderPackRequest>()
        .init_resource::<ShaderPackStatus>()
        .init_resource::<ShaderPackFrameInputs>()
        .init_resource::<LoadedShaderPack>()
        .init_resource::<Loader>()
        .add_plugins((
            ExtractComponentPlugin::<ShaderPackRendering>::default(),
            ExtractResourcePlugin::<LoadedShaderPack>::default(),
            ExtractResourcePlugin::<ShaderPackFrameInputs>::default(),
        ))
        .add_systems(Update, drive_loader);
}

pub(super) fn finish(app: &mut App) {
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_app.init_resource::<ShaderPackViews>().add_systems(
        Render,
        prepare_views
            .in_set(RenderSystems::PrepareResources)
            .after(bevy::render::view::prepare_view_targets),
    );
    install_graph(render_app.world_mut());
}
