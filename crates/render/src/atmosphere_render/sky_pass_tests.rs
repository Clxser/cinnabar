//! Exercises sky/world composition through the real headless render app.

use bevy::render::render_phase::AddRenderCommand;
use std::collections::HashMap;
use std::sync::Mutex;

use bevy::{
    asset::{AssetId, AssetPlugin, uuid_handle},
    camera::{CameraPlugin, RenderTarget},
    core_pipeline::{
        Core3d, Core3dSystems, CorePipelinePlugin,
        core_3d::{
            CORE_3D_DEPTH_FORMAT, Opaque3d, Opaque3dBatchSetKey, Opaque3dBinKey, Transparent3d,
            TransparentSortingInfo3d,
        },
    },
    ecs::{query::Has, query::ROQueryItem, system::SystemParamItem},
    image::ImagePlugin,
    mesh::{Mesh, MeshPlugin},
    prelude::*,
    render::{
        Render, RenderApp, RenderPlugin, RenderSystems,
        camera::ExtractedCamera,
        render_asset::RenderAssets,
        render_phase::{
            BinnedRenderPhaseType, DrawFunctions, InputUniformIndex, PhaseItem,
            PhaseItemExtraIndex, RenderCommand, RenderCommandResult, SetItemPipeline,
            TrackedRenderPass, ViewBinnedRenderPhases, ViewSortedRenderPhases,
        },
        render_resource::*,
        renderer::{
            RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue, ViewQuery,
        },
        settings::RenderCreation,
        sync_world::MainEntity,
        texture::GpuImage,
        view::ExtractedView,
    },
    window::WindowPlugin,
};

use {
    crate::ChunkRenderPlugin,
    render_scene::{AtmosphereFrame, AtmosphereTextureAssets, SkyKind},
};

const SIZE: [u32; 2] = [64, 48];
const WORLD_SHADER: Handle<Shader> = uuid_handle!("c22977be-f4db-4e14-aedb-d31025ce01ca");
/// Clip-space layers at fixed depths, so each image depends on the camera only through the sky.
const WORLD_SOURCE: &str = r#"
fn at(xy: vec2<f32>, distance: f32) -> vec4<f32> { return vec4(xy, NEAR_PLANE / distance, 1.0); }
@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    return at(vec2(f32(index & 1u), f32((index >> 1u) & 1u)) * 4.0 - vec2(1.0), WORLD_DISTANCE);
}
@vertex fn hillside_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    return at(array(vec2(-1.0, -1.0), vec2(1.4, -1.0), vec2(-1.0, 0.6))[index], 20.0);
}
@vertex fn paint_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    return at(array(vec2(-1.0, 1.0), vec2(0.2, 1.0), vec2(-1.0, 0.75))[index], 20.0);
}
@vertex fn glass_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    return at(array(vec2(-0.6, -0.4), vec2(0.9, 0.9), vec2(-0.9, 0.95))[index], 8.0);
}
@fragment fn world() -> @location(0) vec4<f32> { return vec4(0.0, 1.0, 0.0, 1.0); }
@fragment fn hillside(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4(position.x / 64.0, 0.5, position.y / 48.0, 1.0);
}
@fragment fn paint() -> @location(0) vec4<f32> { return vec4(1.0, 1.0, 0.0, 1.0); }
@fragment fn glass() -> @location(0) vec4<f32> { return vec4(0.2, 0.4, 0.9, 0.5); }
"#;

/// One synthetic draw: its geometry, what it writes and the phase that orders it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Layer {
    /// Opaque geometry over the whole view at the app's world distance.
    World,
    /// Opaque geometry below a slanted horizon whose edge splits multisampled pixels.
    Hillside,
    /// Opaque colour without depth over sky that no geometry covers.
    Paint,
    /// An opaque bin that writes no depth and draws nothing, which holds the sky before geometry.
    Inert,
    /// Depth-writing blended surface, like water, straddling the horizon in sorted transparency.
    Glass,
}

impl Layer {
    /// Vertex and fragment entries, depth writes and blending.
    fn raster(self) -> (&'static str, &'static str, bool, Option<BlendState>) {
        match self {
            Self::World => ("vertex", "world", true, None),
            Self::Hillside => ("hillside_vertex", "hillside", true, None),
            Self::Paint => ("paint_vertex", "paint", false, None),
            Self::Inert => ("vertex", "paint", false, None),
            Self::Glass => (
                "glass_vertex",
                "glass",
                true,
                Some(BlendState::ALPHA_BLENDING),
            ),
        }
    }
}

/// The synthetic draws queued for every view.
#[derive(Resource, Clone)]
struct Layers(Vec<Layer>);

/// The sky slot each rendered frame chose, recorded between the opaque and transparent passes.
#[derive(Resource, Default)]
struct SkySlots(Mutex<Vec<bool>>);

struct Fixture {
    gpu: crate::gpu_snapshot::Gpu,
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
}

/// Retains the adapter and instance required to give Bevy the same native test device, with the
/// indirect-draw features that select production's GPU-culled terrain where the adapter has them.
fn fixture() -> Option<Fixture> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = match bevy::tasks::block_on(instance.request_adapter(&default())) {
        Ok(adapter) if adapter.get_info().backend != wgpu::Backend::Noop => adapter,
        Ok(_) | Err(wgpu::RequestAdapterError::NotFound { .. }) => {
            eprintln!("skipping finite skybox real render schedule: missing native GPU fixture");
            return None;
        }
        Err(error) => panic!("finite skybox render schedule adapter: {error}"),
    };
    eprintln!("sky render schedule adapter: {:?}", adapter.get_info().name);
    let (device, queue) = bevy::tasks::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features()
            & (wgpu::Features::INDIRECT_FIRST_INSTANCE | wgpu::Features::MULTI_DRAW_INDIRECT_COUNT),
        required_limits: wgpu::Limits {
            max_storage_buffers_per_shader_stage: crate::required_vertex_storage_buffers(),
            ..default()
        },
        ..default()
    }))
    .expect("finite skybox render schedule device");
    Some(Fixture {
        gpu: crate::gpu_snapshot::Gpu {
            device,
            queue,
            backend: adapter.get_info().backend,
        },
        instance,
        adapter,
    })
}

/// Builds the real sky schedule with the given synthetic layers, the world layer at `distance`.
fn app(
    fixture: &Fixture,
    msaa: Msaa,
    distance: f32,
    layers: &[Layer],
    pack: bool,
) -> (App, Entity, Handle<Image>) {
    let gpu = &fixture.gpu;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(WindowPlugin {
            primary_window: None,
            ..default()
        })
        .add_plugins((AssetPlugin::default(), TransformPlugin))
        .add_plugins(RenderPlugin {
            render_creation: RenderCreation::manual(
                RenderDevice::from(gpu.device.clone()),
                RenderQueue::new(gpu.queue.clone()),
                RenderAdapterInfo::new(fixture.adapter.get_info()),
                RenderAdapter::new(fixture.adapter.clone()),
                RenderInstance::new(fixture.instance.clone()),
            ),
            synchronous_pipeline_compilation: true,
            ..default()
        })
        .add_plugins((
            ImagePlugin::default(),
            MeshPlugin,
            CameraPlugin,
            CorePipelinePlugin,
        ))
        .add_plugins(ChunkRenderPlugin::new(1));
    let source = WORLD_SOURCE
        .replace(
            "NEAR_PLANE",
            &format!("{:?}", render_api::CAMERA_NEAR_PLANE_BLOCKS),
        )
        .replace("WORLD_DISTANCE", &format!("{distance:?}"));
    app.world_mut()
        .resource_mut::<Assets<Shader>>()
        .insert(
            WORLD_SHADER.id(),
            crate::shader_safety::from_wgsl(source, "synthetic world layers"),
        )
        .unwrap();
    app.sub_app_mut(RenderApp)
        .insert_resource(Layers(layers.to_vec()))
        .init_resource::<SkySlots>()
        .add_render_command::<Opaque3d, (SetItemPipeline, DrawTriangle)>()
        .add_render_command::<Opaque3d, (SetItemPipeline, DrawNothing)>()
        .add_render_command::<Transparent3d, (SetItemPipeline, DrawTriangle)>()
        .add_systems(
            Render,
            queue_layers
                .before(super::queue_atmosphere)
                .in_set(RenderSystems::Queue),
        );
    let mut image =
        Image::new_target_texture(SIZE[0], SIZE[1], TextureFormat::Bgra8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let image = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let camera = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            Camera {
                clear_color: bevy::camera::ClearColorConfig::Custom(Color::srgb(1.0, 0.0, 1.0)),
                ..default()
            },
            RenderTarget::Image(image.clone().into()),
            msaa,
            Transform::default(),
            Projection::Perspective(PerspectiveProjection {
                near: render_api::CAMERA_NEAR_PLANE_BLOCKS,
                ..default()
            }),
        ))
        .id();
    let skyboxes = assets::SkyboxDimension::ALL.map(|_| assets::PackSkybox {
        faces: std::array::from_fn(|_| {
            pack.then_some(assets::SkyboxFace {
                width: 1,
                height: 1,
                rgba8: Box::new([255, 0, 0, 255]),
            })
        }),
    });
    app.insert_resource(AtmosphereFrame::default().with_sky_kind(SkyKind::End));
    app.insert_resource(
        AtmosphereTextureAssets::new(super::tests::synthetic_runtime(0x20), [0x51; 32])
            .with_skyboxes(skyboxes, [0x52; 32]),
    );
    app.finish();
    app.cleanup();
    app.sub_app_mut(RenderApp)
        .world_mut()
        .schedule_scope(Core3d, |_, schedule| {
            schedule.add_systems(
                record_slot
                    .after(crate::scene_target::ScenePass::Opaque)
                    .before(crate::scene_target::ScenePass::Transparent)
                    .in_set(Core3dSystems::MainPass),
            );
        });
    (app, camera, image)
}

/// Records the production slot decision for the frame being rendered.
fn record_slot(
    world: &World,
    view: ViewQuery<(
        &ExtractedView,
        &ExtractedCamera,
        &Msaa,
        Has<render_scene::EnhancedRendering>,
    )>,
) {
    let entity = view.entity();
    let (view, camera, msaa, enhanced) = view.into_inner();
    let after = super::sky_pass::after_opaque(world, entity, view, *msaa, camera.hdr, enhanced);
    world.resource::<SkySlots>().0.lock().unwrap().push(after);
}

/// Queues every layer each frame, opaque layers binned and glass through sorted transparency.
fn queue_layers(
    cache: Res<PipelineCache>,
    layers: Res<Layers>,
    mut opaque: ResMut<ViewBinnedRenderPhases<Opaque3d>>,
    mut transparent: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    opaque_functions: Res<DrawFunctions<Opaque3d>>,
    transparent_functions: Res<DrawFunctions<Transparent3d>>,
    views: Query<(Entity, &MainEntity, &ExtractedView, &ExtractedCamera, &Msaa)>,
    mut pipelines: Local<HashMap<(Layer, Msaa, bool), CachedRenderPipelineId>>,
) {
    let triangle = opaque_functions
        .read()
        .id::<(SetItemPipeline, DrawTriangle)>();
    let nothing = opaque_functions
        .read()
        .id::<(SetItemPipeline, DrawNothing)>();
    let glass = transparent_functions
        .read()
        .id::<(SetItemPipeline, DrawTriangle)>();
    for (entity, main, view, camera, msaa) in &views {
        for &layer in &layers.0 {
            let pipeline = *pipelines
                .entry((layer, *msaa, camera.hdr))
                .or_insert_with(|| {
                    cache.queue_render_pipeline(descriptor(layer, *msaa, camera.hdr))
                });
            if layer == Layer::Glass {
                let Some(phase) = transparent.get_mut(&view.retained_view_entity) else {
                    continue;
                };
                crate::transparent_phase::add(
                    phase,
                    Transparent3d {
                        sorting_info: TransparentSortingInfo3d::AlwaysOnTop,
                        distance: 0.0,
                        pipeline,
                        entity: (entity, *main),
                        draw_function: glass,
                        batch_range: 0..1,
                        extra_index: PhaseItemExtraIndex::None,
                        indexed: false,
                    },
                );
                continue;
            }
            let Some(phase) = opaque.get_mut(&view.retained_view_entity) else {
                continue;
            };
            phase.add(
                Opaque3dBatchSetKey {
                    draw_function: if layer == Layer::Inert {
                        nothing
                    } else {
                        triangle
                    },
                    pipeline,
                    material_bind_group_index: None,
                    lightmap_slab: None,
                    slabs: default(),
                },
                Opaque3dBinKey {
                    asset_id: AssetId::<Mesh>::default().untyped(),
                },
                (entity, *main),
                InputUniformIndex::default(),
                BinnedRenderPhaseType::NonMesh,
            );
        }
    }
}

/// The synthetic layer's pipeline for one view configuration.
fn descriptor(layer: Layer, msaa: Msaa, hdr: bool) -> RenderPipelineDescriptor {
    let (vertex, fragment, depth_write, blend) = layer.raster();
    RenderPipelineDescriptor {
        label: Some("synthetic world layer".into()),
        vertex: VertexState {
            shader: WORLD_SHADER,
            entry_point: Some(vertex.into()),
            ..default()
        },
        fragment: Some(FragmentState {
            shader: WORLD_SHADER,
            entry_point: Some(fragment.into()),
            targets: vec![Some(ColorTargetState {
                format: if hdr {
                    render_scene::SCENE_HDR_FORMAT
                } else {
                    render_scene::SCENE_COLOR_FORMAT
                },
                blend,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        depth_stencil: Some(DepthStencilState {
            format: CORE_3D_DEPTH_FORMAT,
            depth_write_enabled: Some(depth_write),
            depth_compare: Some(CompareFunction::GreaterEqual),
            stencil: default(),
            bias: default(),
        }),
        multisample: MultisampleState {
            count: msaa.samples(),
            ..default()
        },
        ..default()
    }
}

struct DrawTriangle;
impl<P: PhaseItem> RenderCommand<P> for DrawTriangle {
    type Param = ();
    type ViewQuery = ();
    type ItemQuery = ();
    fn render<'w>(
        _: &P,
        _: ROQueryItem<'w, '_, Self::ViewQuery>,
        _: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        _: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        pass.draw(0..3, 0..1);
        RenderCommandResult::Success
    }
}

struct DrawNothing;
impl<P: PhaseItem> RenderCommand<P> for DrawNothing {
    type Param = ();
    type ViewQuery = ();
    type ItemQuery = ();
    fn render<'w>(
        _: &P,
        _: ROQueryItem<'w, '_, Self::ViewQuery>,
        _: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        _: SystemParamItem<'w, '_, Self::Param>,
        _: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        RenderCommandResult::Success
    }
}

/// Reads the completed image after the actual production render schedule ran.
fn pixels(app: &App, image: &Handle<Image>) -> Vec<u8> {
    let world = app.sub_app(RenderApp).world();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let images = world.resource::<RenderAssets<GpuImage>>();
    let image = images.get(image).expect("prepared headless image");
    let row = SIZE[0] * 4;
    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some("sky/world regression readback"),
        size: u64::from(row * SIZE[1]),
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&default());
    encoder.copy_texture_to_buffer(
        image.texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(SIZE[1]),
            },
        },
        Extent3d {
            width: SIZE[0],
            height: SIZE[1],
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    buffer.slice(..).get_mapped_range().unwrap().to_vec()
}

/// The centre pixel, as BGRA bytes.
fn pixel(app: &App, image: &Handle<Image>) -> [u8; 4] {
    let offset = ((SIZE[1] / 2 * SIZE[0] + SIZE[0] / 2) * 4) as usize;
    pixels(app, image)[offset..offset + 4].try_into().unwrap()
}

/// Renders settled frames and returns the image with the last frame's sky slot.
fn capture(app: &mut App, image: &Handle<Image>) -> (Vec<u8>, bool) {
    for _ in 0..4 {
        app.update();
    }
    let slot = app
        .sub_app(RenderApp)
        .world()
        .resource::<SkySlots>()
        .0
        .lock()
        .unwrap()
        .last()
        .copied()
        .expect("a rendered frame recorded its sky slot");
    (pixels(app, image), slot)
}

#[test]
fn end_skybox_never_overwrites_near_or_distant_opaque_geometry() {
    let Some(gpu) = fixture() else {
        return;
    };
    for msaa in [Msaa::Off, Msaa::Sample4] {
        let (mut sky, _, image) = app(&gpu, msaa, 4.0, &[], true);
        for _ in 0..8 {
            sky.update();
        }
        let colour = pixel(&sky, &image);
        assert!(
            colour[2] > 200 && colour[0] < 20 && colour[1] < 20,
            "{msaa:?} sky-only control did not draw the red pack cube: {colour:?}"
        );
        drop(sky);
        for distance in [4.0, 100.0] {
            let (mut app, _, image) = app(&gpu, msaa, distance, &[Layer::World], true);
            for _ in 0..8 {
                app.update();
            }
            let colour = pixel(&app, &image);
            assert!(
                colour[1] > 200 && colour[0] < 20 && colour[2] < 20,
                "{msaa:?} world at {distance} blocks was covered by sky: {colour:?}"
            );
        }
    }
}

/// Covers the late stock path through the actual schedule in every dimension and MSAA mode.
#[test]
fn stock_sky_fills_clear_samples_without_covering_opaque_geometry() {
    let Some(gpu) = fixture() else {
        return;
    };
    for msaa in [Msaa::Off, Msaa::Sample4] {
        for kind in [SkyKind::Overworld, SkyKind::Nether, SkyKind::End] {
            for distance in [None, Some(4.0), Some(100.0)] {
                let layers = distance.map_or(Vec::new(), |_| vec![Layer::World]);
                let (mut app, _, image) = app(&gpu, msaa, distance.unwrap_or(4.0), &layers, false);
                app.insert_resource(AtmosphereFrame::default().with_sky_kind(kind));
                for _ in 0..8 {
                    app.update();
                }
                let colour = pixel(&app, &image);
                if distance.is_some() {
                    assert!(
                        colour[1] > 200 && colour[0] < 20 && colour[2] < 20,
                        "{msaa:?}/{kind:?} sky overwrote the world at {distance:?}: {colour:?}"
                    );
                } else {
                    assert!(
                        colour[..3] != [255, 0, 255],
                        "{msaa:?}/{kind:?} stock sky left clear samples unfilled: {colour:?}"
                    );
                }
            }
        }
    }
}

/// Every sky the frame can show, with the camera turned toward its sun, moon and stars.
fn skies() -> Vec<(&'static str, AtmosphereFrame)> {
    let fog = assets::ResolvedFog {
        start: 8.0,
        end: 64.0,
        rgb: [0.8, 0.35, 0.2],
    };
    vec![
        (
            "dawn",
            AtmosphereFrame::from_bedrock_time(23_500.0, 0.0, 0.0),
        ),
        (
            "noon",
            AtmosphereFrame::from_bedrock_time(6_000.0, 0.0, 0.0),
        ),
        (
            "night",
            AtmosphereFrame::from_bedrock_time(18_000.0, 0.0, 0.0),
        ),
        (
            "storm",
            AtmosphereFrame::from_bedrock_time(12_500.0, 1.0, 1.0),
        ),
        (
            "fog",
            AtmosphereFrame::from_bedrock_time(13_000.0, 0.0, 0.0)
                .with_environment_profile(None, Some(fog)),
        ),
        (
            "underwater",
            AtmosphereFrame::from_bedrock_time(6_000.0, 0.0, 0.0)
                .with_camera_medium(meshing::CameraMedium::Water),
        ),
        (
            "nether",
            AtmosphereFrame::default().with_sky_kind(SkyKind::Nether),
        ),
        (
            "end",
            AtmosphereFrame::default().with_sky_kind(SkyKind::End),
        ),
    ]
}

/// Bytes that differ between two equally sized images, with the first differing pixel.
fn difference(left: &[u8], right: &[u8]) -> (usize, Option<usize>) {
    let pixels = left.chunks_exact(4).zip(right.chunks_exact(4));
    let differing: Vec<usize> = pixels
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(index, _)| index)
        .collect();
    (differing.len(), differing.first().copied())
}

/// Drawing the sky after opaque geometry leaves every sample of the image as drawing it first:
/// the multisampled horizon edge, sun, moon, stars, weather and fog skies, other dimensions, the
/// pack cube, and depth-writing blended glass over both sky and ground.
#[test]
fn sky_after_opaque_geometry_matches_the_sky_drawn_first() {
    let Some(gpu) = fixture() else {
        return;
    };
    let directions = [
        Quat::IDENTITY,
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2),
        Quat::from_rotation_x(1.2),
    ];
    let scene = [Layer::Hillside, Layer::Glass];
    let held = [Layer::Hillside, Layer::Glass, Layer::Inert];
    for msaa in [Msaa::Off, Msaa::Sample2, Msaa::Sample4] {
        for pack in [false, true] {
            let (mut app, camera, image) = app(&gpu, msaa, 100.0, &scene, pack);
            for (name, frame) in skies() {
                app.insert_resource(frame);
                for (index, rotation) in directions.into_iter().enumerate() {
                    *app.world_mut().get_mut::<Transform>(camera).unwrap() =
                        Transform::from_rotation(rotation);
                    let case = format!("{msaa:?}, pack {pack}, {name} sky, direction {index}");
                    app.sub_app_mut(RenderApp)
                        .insert_resource(Layers(scene.to_vec()));
                    let (after, late) = capture(&mut app, &image);
                    assert!(late, "{case}: opaque geometry must admit the later sky");
                    app.sub_app_mut(RenderApp)
                        .insert_resource(Layers(held.to_vec()));
                    let (first, late) = capture(&mut app, &image);
                    assert!(
                        !late,
                        "{case}: a depth-free opaque bin must hold the sky first"
                    );
                    let (differing, first_index) = difference(&after, &first);
                    assert_eq!(
                        differing, 0,
                        "{case}: {differing} pixels changed, first at {first_index:?}"
                    );
                    let corner = ((SIZE[0] + SIZE[0] - 4) * 4) as usize;
                    assert_ne!(
                        after[corner..corner + 3],
                        [255, 0, 255],
                        "{case}: the sky must fill the corner no geometry covers"
                    );
                }
            }
        }
    }
}

/// Opaque colour written without depth over open sky keeps the sky beneath it, so the sky is
/// drawn before world geometry whenever such a draw is queued.
#[test]
fn opaque_colour_without_depth_holds_the_sky_before_world_geometry() {
    let Some(gpu) = fixture() else {
        return;
    };
    for msaa in [Msaa::Off, Msaa::Sample4] {
        let (mut app, _, image) = app(&gpu, msaa, 100.0, &[Layer::Hillside, Layer::Paint], false);
        app.insert_resource(AtmosphereFrame::from_bedrock_time(6_000.0, 0.0, 0.0));
        let (image, late) = capture(&mut app, &image);
        assert!(
            !late,
            "{msaa:?}: the depth-free paint must hold the sky first"
        );
        // The stock tonemapper leaves yellow paint yellow, not exactly saturated.
        let paint = &image[((SIZE[0] + 2) * 4) as usize..][..3];
        assert!(
            paint[0] < 20 && paint[1] > 200 && paint[2] > 200,
            "{msaa:?}: the sky covered paint drawn over it: {paint:?}"
        );
    }
}

/// The sky may follow only geometry that gives every sample it colours a depth.
#[test]
fn only_pipelines_writing_colour_and_depth_together_cover_the_sky() {
    use super::sky_pass::covers_sky;
    let covering = descriptor(Layer::Hillside, Msaa::Sample4, false);
    assert!(covers_sky(&covering));
    let mut rgb = covering.clone();
    rgb.fragment.as_mut().unwrap().targets[0]
        .as_mut()
        .unwrap()
        .write_mask = ColorWrites::COLOR;
    assert!(covers_sky(&rgb), "alpha is the clear value beneath both");
    for (case, descriptor) in [
        (
            "no depth writes",
            descriptor(Layer::Paint, Msaa::Off, false),
        ),
        ("blending", descriptor(Layer::Glass, Msaa::Off, false)),
        ("depth without colour", {
            let mut descriptor = covering.clone();
            descriptor.fragment.as_mut().unwrap().targets[0]
                .as_mut()
                .unwrap()
                .write_mask = ColorWrites::empty();
            descriptor
        }),
        ("no colour target", {
            let mut descriptor = covering.clone();
            descriptor.fragment.as_mut().unwrap().targets.clear();
            descriptor
        }),
        (
            "no fragment stage",
            RenderPipelineDescriptor {
                fragment: None,
                ..covering.clone()
            },
        ),
        (
            "no depth attachment",
            RenderPipelineDescriptor {
                depth_stencil: None,
                ..covering.clone()
            },
        ),
    ] {
        assert!(!covers_sky(&descriptor), "{case} must not cover the sky");
    }
}

/// Installed world assets and their registry, or `None` with a named missing-fixture message.
fn world_fixture() -> Option<(
    Vec<assets::RegistryRecord>,
    std::sync::Arc<assets::RuntimeAssets>,
)> {
    let target: serde_json::Value =
        serde_json::from_slice(include_bytes!("../../../../assets/bedrock-target.json")).unwrap();
    let path = env_compat::var_os(assets::carriers::startup::ASSET_PATH_ENVIRONMENT)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(target["artifacts"]["world_assets"].as_str().unwrap())
        });
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "missing fixture: sky ordering world carrier {}",
                path.display()
            );
            return None;
        }
        Err(error) => panic!("sky ordering world carrier {}: {error}", path.display()),
    };
    let records = assets::read_registry_for_protocol(
        include_bytes!("../../../assets/data/block-registry-v2193.bin"),
        target["wire_protocol"].as_u64().unwrap() as u32,
    )
    .unwrap()
    .into_vec();
    Some((
        records,
        std::sync::Arc::new(assets::RuntimeAssets::decode(&bytes).unwrap()),
    ))
}

/// A stepped stone ridge whose top edges meet the sky, with leaves, stained glass and a water
/// source floating in open air above it, meshed by the production mesher.
fn ridge(records: &[assets::RegistryRecord], assets: &assets::RuntimeAssets) -> meshing::ChunkMesh {
    let id = |name: &str| {
        records
            .iter()
            .find(|record| record.name.as_ref() == name)
            .unwrap_or_else(|| panic!("{name} is registered"))
            .sequential_id
    };
    let air = id("minecraft:air");
    let key = world::SubChunkKey::new(0, 0, 0, 0);
    let mut store = world::ChunkStore::new();
    store.mark_sub_chunk_loaded(key).unwrap();
    let mut place = |[x, y, z]: [u8; 3], block| {
        store
            .update_block(key, world::BlockUpdate::new(x, y, z, 0, block), air)
            .unwrap();
    };
    let stone = id("minecraft:stone");
    for x in 0..16 {
        for z in 0..3 {
            for y in 0..4 + (x * 7 % 5) {
                place([x, y, z], stone);
            }
        }
    }
    for x in [9, 10, 11] {
        place([x, 10, 1], id("minecraft:oak_leaves"));
    }
    place([6, 12, 1], id("minecraft:white_stained_glass"));
    place([3, 11, 1], id("minecraft:water"));
    meshing::mesh_sub_chunk(
        &meshing::BlockClassifier::new(air),
        assets,
        assets::NetworkIdMode::Sequential,
        &meshing::Neighbourhood::empty(),
        &store.sub_chunk(key).unwrap(),
    )
}

/// Real terrain, cutout leaves, blended glass and water against every sky, fogged at short
/// range, renders the same pixels with the sky drawn after opaque geometry as before it.
#[test]
fn sky_after_real_terrain_matches_the_sky_drawn_first() {
    let Some((records, assets)) = world_fixture() else {
        return;
    };
    let Some(gpu) = fixture() else {
        return;
    };
    let mesh = ridge(&records, &assets);
    let fog = assets::ResolvedFog {
        start: 6.0,
        end: 30.0,
        rgb: [0.7, 0.75, 0.9],
    };
    let skies = [
        (
            "fogged day",
            AtmosphereFrame::from_bedrock_time(6_000.0, 0.0, 0.0)
                .with_environment_profile(None, Some(fog)),
        ),
        (
            "night",
            AtmosphereFrame::from_bedrock_time(18_000.0, 0.0, 0.0),
        ),
        (
            "storm",
            AtmosphereFrame::from_bedrock_time(12_500.0, 1.0, 1.0),
        ),
        (
            "nether",
            AtmosphereFrame::default().with_sky_kind(SkyKind::Nether),
        ),
    ];
    for msaa in [Msaa::Off, Msaa::Sample2, Msaa::Sample4] {
        let (mut app, camera, image) = app(&gpu, msaa, 100.0, &[], false);
        app.insert_resource(render_scene::ChunkTextureAssets::new(assets.clone()));
        *app.world_mut().get_mut::<Transform>(camera).unwrap() =
            Transform::from_xyz(8.0, 3.0, 16.0).looking_at(Vec3::new(8.0, 9.0, 0.0), Vec3::Y);
        let empty: Vec<_> = skies
            .iter()
            .map(|(_, frame)| {
                app.insert_resource(*frame);
                capture(&mut app, &image).0
            })
            .collect();
        app.world_mut()
            .resource_mut::<render_scene::ChunkRenderQueue>()
            .try_insert(
                world::SubChunkKey::new(0, 0, 0, 0),
                mesh.clone(),
                render_scene::ChunkUploadPriority::new(0.0),
            )
            .unwrap();
        for _ in 0..12 {
            app.update();
        }
        for ((name, frame), empty) in skies.iter().zip(&empty) {
            let case = format!("{msaa:?}, {name}");
            app.insert_resource(*frame);
            app.sub_app_mut(RenderApp)
                .insert_resource(Layers(Vec::new()));
            let (after, late) = capture(&mut app, &image);
            assert!(late, "{case}: terrain must admit the later sky");
            app.sub_app_mut(RenderApp)
                .insert_resource(Layers(vec![Layer::Inert]));
            let (first, late) = capture(&mut app, &image);
            assert!(
                !late,
                "{case}: a depth-free opaque bin must hold the sky first"
            );
            let (differing, first_index) = difference(&after, &first);
            assert_eq!(
                differing, 0,
                "{case}: {differing} pixels changed, first at {first_index:?}"
            );
            assert!(
                difference(&after, empty).0 > 200,
                "{case}: the ridge must reach the image"
            );
        }
    }
}
