//! Worker-owned native replay rendering on a shared device, with independent scene resources.

use std::{collections::BTreeMap, sync::Arc};

use bevy::{
    asset::AssetPlugin,
    camera::{CameraPlugin, ManualTextureViewHandle, RenderTarget},
    core_pipeline::{CorePipelinePlugin, tonemapping::Tonemapping},
    mesh::MeshPlugin,
    prelude::*,
    render::{
        RenderApp, RenderPlugin,
        render_resource::{CachedPipelineState, PipelineCache, TextureViewDescriptor},
        renderer::{RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue},
        settings::RenderCreation,
        texture::{ManualTextureView, ManualTextureViews},
    },
};
use recording_scene::{playback::SceneFrame, scene::ChunkId};
use recording_studio::camera::CameraPose;
use render_scene::{
    ChunkAnimationClock,
    ui_preview::{PreviewImage, PreviewTarget},
};

mod atmosphere;
mod effects;
mod faults;
mod hud;
mod items;
mod labels;
mod particles;
pub use effects::FrameEffects;
mod readback;
pub use readback::ReplayReadback;
mod actor_frame;
mod compositor;
mod scene;
mod targets;
mod visibility;
mod windowless;
pub use compositor::{CompositeInput, CompositeLayer, FrameCompositor};
mod titles;
pub use titles::{FrameTitles, TitleLayer};

const TARGET: ManualTextureViewHandle = ManualTextureViewHandle(0);

#[derive(Clone, Resource)]
pub struct ReplayRenderContext {
    device: RenderDevice,
    queue: RenderQueue,
    info: RenderAdapterInfo,
    adapter: RenderAdapter,
    instance: RenderInstance,
    faults: faults::DeviceFaults,
}

pub struct ReplayPreviewPlugin;

impl Plugin for ReplayPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<faults::DeviceFaults>()
            .add_systems(First, faults::exit_on_fault);
    }

    /// The instance lives only in RenderApp; install its shared context after renderer initialization.
    fn finish(&self, app: &mut App) {
        let Some(render) = app.get_sub_app(RenderApp) else {
            return;
        };
        let world = render.world();
        let (Some(device), Some(queue), Some(info), Some(adapter), Some(instance)) = (
            world.get_resource::<RenderDevice>(),
            world.get_resource::<RenderQueue>(),
            world.get_resource::<RenderAdapterInfo>(),
            world.get_resource::<RenderAdapter>(),
            world.get_resource::<RenderInstance>(),
        ) else {
            return;
        };
        let context = ReplayRenderContext::new(
            device.clone(),
            queue.clone(),
            info.clone(),
            adapter.clone(),
            instance.clone(),
            app.world().resource::<faults::DeviceFaults>().clone(),
        );
        app.insert_resource(context);
    }
}

impl ReplayRenderContext {
    /// Clones device handles only; the replay renderer creates no network or live scene authority.
    fn new(
        device: RenderDevice,
        queue: RenderQueue,
        info: RenderAdapterInfo,
        adapter: RenderAdapter,
        instance: RenderInstance,
        faults: faults::DeviceFaults,
    ) -> Self {
        Self {
            device,
            queue,
            info,
            adapter,
            instance,
            faults,
        }
    }
}

pub struct ReplayRenderer {
    app: App,
    context: ReplayRenderContext,
    targets: targets::Targets,
    drawing: Option<Arc<PreviewTarget>>,
    camera: Entity,
    chunks: BTreeMap<ChunkId, Entity>,
    palette: Option<recording_studio::object_store::AssetId>,
    environment: Option<recording_studio::object_store::AssetId>,
    hud_palette: Option<[u8; 32]>,
    atmosphere_visual_assets: Option<Arc<recording_scene::visuals::atmosphere::AtmosphereAssets>>,
    actor_publication: actor_frame::Publication,
    actor_intersection: visibility::IntersectionCache,
    terrain_dirty: bool,
    generation: u64,
    size: [u32; 2],
}

#[derive(Resource, Default)]
struct RecordedClock(ChunkAnimationClock);

impl ReplayRenderer {
    /// Constructs a separate headless app on its calling worker. Never call from the input thread.
    pub fn new(context: ReplayRenderContext, size: [u32; 2]) -> Result<Self, String> {
        let targets = targets::Targets::new(&context.device, size)?;
        let callbacks = faults::DeviceCallbackGuard::new(&context.device, &context.faults);
        let mut app = App::new();
        app.insert_resource(context.faults.clone());
        app.insert_resource(bevy::render::error_handler::RenderErrorHandler(
            faults::initialization_error,
        ));
        app.add_plugins(MinimalPlugins)
            .add_plugins(windowless::plugin())
            .add_plugins(AssetPlugin::default())
            .add_plugins(RenderPlugin {
                render_creation: RenderCreation::manual(
                    context.device.clone(),
                    context.queue.clone(),
                    context.info.clone(),
                    context.adapter.clone(),
                    context.instance.clone(),
                ),
                ..default()
            })
            .add_plugins((
                ImagePlugin::default(),
                MeshPlugin,
                CameraPlugin,
                TransformPlugin,
                CorePipelinePlugin,
                crate::ChunkRenderPlugin::default(),
                crate::ActorRenderPlugin,
                crate::ParticleRenderPlugin,
                crate::BlockEntityRenderPlugin,
                crate::DroppedItemRenderPlugin,
                crate::HandRigRenderPlugin,
                crate::UiRenderPlugin,
                crate::ScreenOverlayRenderPlugin,
                crate::EnhancedRenderPlugin,
            ))
            .init_resource::<RecordedClock>()
            .add_systems(Last, restore_clock);
        let camera = camera(app.world_mut());
        app.finish();
        app.cleanup();
        drop(callbacks);
        Ok(Self {
            app,
            context,
            targets,
            drawing: None,
            camera,
            chunks: BTreeMap::new(),
            palette: None,
            environment: None,
            hud_palette: None,
            atmosphere_visual_assets: None,
            actor_publication: actor_frame::Publication::default(),
            actor_intersection: visibility::IntersectionCache::default(),
            terrain_dirty: true,
            generation: 0,
            size,
        })
    }

    pub const fn size(&self) -> [u32; 2] {
        self.size
    }

    /// Publishes one fully admitted source frame before drawing. A seek creates fresh view histories.
    pub fn apply(&mut self, frame: &SceneFrame, pose: CameraPose) -> Result<(), String> {
        self.apply_view(frame, pose, pose == frame.view.pose, true, false, None)
    }

    /// Source-camera overlays remain anchored to their recording when editor camera effects move its pose.
    pub fn apply_view(
        &mut self,
        frame: &SceneFrame,
        pose: CameraPose,
        source_view: bool,
        hide_hud: bool,
        hide_hand: bool,
        spectated: Option<recording_studio::camera::CameraTargetRef>,
    ) -> Result<(), String> {
        pose.validate().map_err(|error| error.to_string())?;
        let transform = scene::camera_transform(pose)?;
        if frame.temporal_reset {
            crate::particle_render::reset_replay(self.app.sub_app_mut(RenderApp).world_mut());
            crate::dropped_item_render::replay::reset(self.app.sub_app_mut(RenderApp).world_mut());
            self.app.world_mut().despawn(self.camera);
            self.camera = camera(self.app.world_mut());
        }
        self.apply_scene(frame, pose, source_view, !hide_hand, spectated)?;
        self.apply_labels(frame, transform, source_view);
        self.apply_items(frame, transform, source_view)?;
        self.reproject_particles(frame, transform, pose.fov, source_view);
        self.apply_atmosphere_visuals(frame, transform, source_view)?;
        self.apply_hud(frame, source_view && !hide_hud)?;
        let options = frame.environment.state.view.and_then(|view| view.enhanced);
        let mut camera = self.app.world_mut().entity_mut(self.camera);
        if let Some(options) = options {
            if !render_model::enhanced_rendering_enabled() {
                return Err(
                    "This take requires Enhanced rendering, which is unavailable in this client."
                        .into(),
                );
            }
            camera.insert(render_scene::EnhancedRendering {
                shadows: options.shadows,
                shadow_resolution: options.shadow_resolution,
                shadow_cascades: options.shadow_cascades,
                shadow_distance: options.shadow_distance,
                bloom: options.bloom,
                light_shafts: options.light_shafts,
                waving: options.waving,
                water_reflections: options.water_reflections,
            });
        } else {
            camera.remove::<render_scene::EnhancedRendering>();
        }
        camera.insert((
            transform,
            Projection::Perspective(scene::camera_projection(pose, self.size)),
        ));
        Ok(())
    }

    /// Advances preparation without GPU waits. `None` asks the worker to retry after compilation/upload.
    pub fn draw(&mut self) -> Result<Option<Arc<PreviewImage>>, String> {
        if let Some(error) = self.context.faults.error() {
            return Err(error);
        }
        if self.drawing.is_none() {
            self.drawing = self.targets.acquire(&self.context.device)?;
        }
        let Some(target) = self.drawing.as_ref().cloned() else {
            return Ok(None);
        };
        self.app
            .world_mut()
            .resource_mut::<ManualTextureViews>()
            .insert(
                TARGET,
                ManualTextureView::with_default_format(
                    target
                        .texture()
                        .create_view(&TextureViewDescriptor::default()),
                    UVec2::from_array(self.size),
                ),
            );
        self.app.update();
        // Cancellation or an unready frame may drop its display lease immediately after this call.
        let submitted = Arc::clone(&target);
        self.context
            .queue
            .on_submitted_work_done(move || drop(submitted));
        let render = self.app.sub_app_mut(RenderApp).world_mut();
        let mut shaders_ready = true;
        for pipeline in render.resource::<PipelineCache>().pipelines() {
            if let CachedPipelineState::Err(error) = &pipeline.state {
                if crate::pipeline_warmup::shader_pending(error) {
                    shaders_ready = false;
                    continue;
                }
                return Err(format!("Replay shader preparation failed: {error}"));
            }
        }
        if self.terrain_dirty {
            self.terrain_dirty = !crate::chunk::replay::ready(render);
        }
        let ready = shaders_ready
            && !self.terrain_dirty
            && crate::dropped_item_render::replay::ready(self.app.sub_app(RenderApp).world())?
            && crate::ui_render::replay_ready(self.app.sub_app(RenderApp).world())
            && crate::actor_render::replay_ready(self.app.sub_app(RenderApp).world())
            && self
                .app
                .world()
                .resource::<render_scene::PipelineWarmupReadiness>()
                .is_ready()
            && self
                .app
                .world()
                .resource::<render_scene::ActorPipelineReadiness>()
                .is_ready();
        if !ready {
            return Ok(None);
        }
        self.drawing = None;
        Ok(Some(Arc::new(PreviewImage { target })))
    }
}

/// No window or live camera is inherited by the native replay context.
fn camera(world: &mut World) -> Entity {
    world
        .spawn((
            Camera3d::default(),
            Camera {
                clear_color: Color::BLACK.into(),
                ..default()
            },
            RenderTarget::TextureView(TARGET),
            Msaa::Off,
            Tonemapping::Linear,
        ))
        .id()
}

/// Native terrain's wall-clock update must not change a paused or scrubbed recorded animation.
fn restore_clock(source: Res<RecordedClock>, mut clock: ResMut<ChunkAnimationClock>) {
    *clock = source.0;
}
