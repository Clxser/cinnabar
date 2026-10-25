//! Real render-app frames with transparent occlusion on and off are pixel-identical: water behind
//! an opaque wall, water partly hidden at the screen edge, water behind stained glass, ice beside
//! water, an underwater eye and a camera moving between views, all with 2x MSAA.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use bevy::{
    asset::AssetPlugin,
    camera::{Camera3dDepthTextureUsage, CameraPlugin, RenderTarget},
    core_pipeline::{CorePipelinePlugin, tonemapping::Tonemapping},
    image::ImagePlugin,
    mesh::MeshPlugin,
    prelude::*,
    render::{
        RenderApp, RenderPlugin,
        gpu_readback::{Readback, ReadbackComplete},
        render_resource::{CachedPipelineState, PipelineCache, TextureFormat, TextureUsages},
        renderer::{RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue},
        settings::RenderCreation,
    },
    window::WindowPlugin,
};

use super::GpuCull;
use crate::chunk::transparent::submissions::TransparentSubmissions;

const SIZE: [u32; 2] = [192, 144];
/// Frames that let uploads, sorts and the cull history settle after a change.
const SETTLE_FRAMES: usize = 10;

#[derive(Resource, Default)]
struct Capture(Vec<u8>);

/// The installed world carrier and block registry, or `None` with a named missing fixture.
fn carriers() -> Option<(Vec<assets::RegistryRecord>, Arc<assets::RuntimeAssets>)> {
    let target: serde_json::Value =
        serde_json::from_slice(include_bytes!("../../../../../assets/bedrock-target.json"))
            .unwrap();
    let path = env_compat::var_os(assets::carriers::startup::ASSET_PATH_ENVIRONMENT)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(target["artifacts"]["world_assets"].as_str().unwrap())
        });
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("missing fixture: world carrier {}", path.display());
            return None;
        }
        Err(error) => panic!("world carrier {}: {error}", path.display()),
    };
    let records = assets::read_registry_for_protocol(
        include_bytes!("../../../../assets/data/block-registry-v2193.bin"),
        target["wire_protocol"].as_u64().unwrap() as u32,
    )
    .unwrap()
    .into_vec();
    Some((
        records,
        Arc::new(assets::RuntimeAssets::decode(&bytes).unwrap()),
    ))
}

/// The adapter, device and queue Bevy renders with, with the features the GPU cull needs.
struct Fixture {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn fixture() -> Option<Fixture> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let options = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..default()
    };
    let adapter = match bevy::tasks::block_on(instance.request_adapter(&options)) {
        Ok(adapter) if adapter.get_info().backend != wgpu::Backend::Noop => adapter,
        Ok(_) | Err(wgpu::RequestAdapterError::NotFound { .. }) => {
            eprintln!("skipping transparent occlusion scene: missing native GPU adapter fixture");
            return None;
        }
        Err(error) => panic!("transparent occlusion scene adapter: {error}"),
    };
    let (device, queue) = bevy::tasks::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features()
            & (wgpu::Features::INDIRECT_FIRST_INSTANCE | wgpu::Features::MULTI_DRAW_INDIRECT_COUNT),
        required_limits: adapter.limits(),
        ..default()
    }))
    .expect("transparent occlusion scene device");
    Some(Fixture {
        instance,
        adapter,
        device,
        queue,
    })
}

/// Sequential ids of the scene's blocks.
struct Blocks {
    air: u32,
    stone: u32,
    water: u32,
    glass: u32,
    ice: u32,
}

impl Blocks {
    fn find(records: &[assets::RegistryRecord]) -> Self {
        let id = |name: &str| {
            records
                .iter()
                .find(|record| record.name.as_ref() == name)
                .unwrap_or_else(|| panic!("{name} is registered"))
                .sequential_id
        };
        Self {
            air: records
                .iter()
                .find(|record| record.flags.contains(assets::BlockFlags::AIR))
                .unwrap()
                .sequential_id,
            stone: id("minecraft:stone"),
            water: id("minecraft:water"),
            glass: id("minecraft:light_blue_stained_glass"),
            ice: id("minecraft:ice"),
        }
    }
}

type Cells = BTreeMap<[i32; 3], Vec<([u8; 3], u32)>>;

/// Fills the inclusive local box `low..=high` of sub-chunk `key` with `id`.
fn fill(cells: &mut Cells, key: [i32; 3], low: [u8; 3], high: [u8; 3], id: u32) {
    let blocks = cells.entry(key).or_default();
    for x in low[0]..=high[0] {
        for y in low[1]..=high[1] {
            for z in low[2]..=high[2] {
                blocks.push(([x, y, z], id));
            }
        }
    }
}

/// A stone wall at z 0 spanning x -16..32, pools on stone floors behind it, a pool at its right
/// end, stained glass and ice in front of a pool before the wall, and a flooded sub-chunk.
fn scene(blocks: &Blocks) -> Cells {
    let mut cells = Cells::new();
    for x in -1..=1 {
        fill(&mut cells, [x, 0, 0], [0, 0, 0], [15, 15, 0], blocks.stone);
        for z in -4..=-1 {
            fill(&mut cells, [x, 0, z], [0, 0, 0], [15, 0, 15], blocks.stone);
            fill(&mut cells, [x, 0, z], [0, 1, 0], [15, 5, 15], blocks.water);
        }
    }
    fill(&mut cells, [2, 0, -1], [0, 0, 0], [15, 0, 15], blocks.stone);
    fill(&mut cells, [2, 0, -1], [0, 1, 0], [15, 5, 15], blocks.water);
    fill(&mut cells, [1, 0, 1], [2, 0, 0], [13, 2, 3], blocks.water);
    fill(&mut cells, [1, 0, 1], [2, 0, 4], [13, 9, 4], blocks.glass);
    fill(&mut cells, [1, 0, 1], [0, 0, 0], [1, 2, 3], blocks.ice);
    fill(&mut cells, [0, 0, 2], [0, 0, 0], [15, 9, 15], blocks.water);
    cells
}

/// Meshes every sub-chunk of `cells` alone, as the production mesher does with no neighbours.
fn meshes(
    assets: &assets::RuntimeAssets,
    air: u32,
    cells: &Cells,
) -> Vec<(world::SubChunkKey, meshing::ChunkMesh)> {
    cells
        .iter()
        .map(|(&[x, y, z], blocks)| {
            let key = world::SubChunkKey::new(0, x, y, z);
            let mut store = world::ChunkStore::new();
            store.mark_sub_chunk_loaded(key).unwrap();
            for &([bx, by, bz], id) in blocks {
                store
                    .update_block(key, world::BlockUpdate::new(bx, by, bz, 0, id), air)
                    .unwrap();
            }
            let mesh = meshing::mesh_sub_chunk(
                &meshing::BlockClassifier::new(air),
                assets,
                assets::NetworkIdMode::Sequential,
                &meshing::Neighbourhood::empty(),
                &store.sub_chunk(key).unwrap(),
            );
            (key, mesh)
        })
        .collect()
}

/// The ordinary LDR world graph on `fixture`'s device, with one 2x MSAA camera.
fn app(fixture: &Fixture, assets: Arc<assets::RuntimeAssets>) -> (App, Entity) {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin))
        .add_plugins(WindowPlugin {
            primary_window: None,
            ..default()
        })
        .add_plugins(AssetPlugin::default())
        .add_plugins(RenderPlugin {
            render_creation: RenderCreation::manual(
                RenderDevice::from(fixture.device.clone()),
                RenderQueue::new(fixture.queue.clone()),
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
        .add_plugins((crate::ChunkRenderPlugin::new(64), crate::AtmospherePlugin))
        .insert_resource(render_scene::ChunkTextureAssets::new(assets))
        .insert_resource(render_scene::AtmosphereFrame::from_bedrock_time(
            2500.0, 0.0, 0.0,
        ))
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::ZERO,
        ))
        .init_resource::<Capture>();
    let mut image = Image::new_target_texture(
        SIZE[0],
        SIZE[1],
        TextureFormat::Rgba8Unorm,
        Some(TextureFormat::Rgba8UnormSrgb),
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let image = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let camera = app
        .world_mut()
        .spawn((
            Camera3d {
                depth_texture_usages: Camera3dDepthTextureUsage::from(
                    TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                ),
                ..default()
            },
            Camera::default(),
            RenderTarget::Image(image.clone().into()),
            Msaa::Sample2,
            Tonemapping::None,
            Transform::default(),
        ))
        .id();
    app.world_mut().spawn(Readback::texture(image)).observe(
        |event: On<ReadbackComplete>, mut capture: ResMut<Capture>| {
            capture.0.clone_from(&event.data);
        },
    );
    app.finish();
    app.cleanup();
    (app, camera)
}

/// Runs `count` frames to completion, rejecting pipeline failures.
fn frames(app: &mut App, count: usize) {
    for _ in 0..count {
        app.update();
        app.sub_app(RenderApp)
            .world()
            .resource::<RenderDevice>()
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
    let cache = app.sub_app(RenderApp).world().resource::<PipelineCache>();
    for pipeline in cache.pipelines() {
        if let CachedPipelineState::Err(error) = &pipeline.state {
            panic!("transparent occlusion scene pipeline: {error}");
        }
    }
}

fn cull(app: &App) -> &GpuCull {
    app.sub_app(RenderApp)
        .world()
        .get_resource::<GpuCull>()
        .expect("the GPU cull is installed on this device")
}

/// Slots the mask's last rewrite hid.
fn hidden_slots(app: &App) -> u32 {
    let world = app.sub_app(RenderApp).world();
    let Some(mask) = cull(app).transparent.mask() else {
        return 0;
    };
    let device = world.resource::<RenderDevice>().wgpu_device();
    let size = mask.texture.size();
    let bytes = u64::from(size.width * 4 * size.height);
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&default());
    encoder.copy_texture_to_buffer(
        mask.texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.width * 4),
                rows_per_image: Some(size.height),
            },
        },
        size,
    );
    world.resource::<RenderQueue>().submit([encoder.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let words = staging
        .slice(..)
        .get_mapped_range()
        .expect("mask readback is mapped");
    bytemuck::cast_slice::<u8, u32>(&words)
        .iter()
        .map(|word| word.count_ones())
        .sum()
}

fn mismatches(a: &[u8], b: &[u8]) -> usize {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(a, b)| a != b)
        .count()
}

/// Writes requested diagnostic frames outside the repository.
fn save(name: &str, pixels: &[u8]) {
    if let Some(directory) = std::env::var_os("CINNABAR_REVIEW_SNAPSHOT_DIR") {
        image::save_buffer(
            PathBuf::from(directory).join(format!("{name}.png")),
            pixels,
            SIZE[0],
            SIZE[1],
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
}

#[test]
fn transparent_occlusion_keeps_real_frames_pixel_identical() {
    let Some((records, assets)) = carriers() else {
        return;
    };
    let Some(fixture) = fixture() else {
        return;
    };
    eprintln!(
        "transparent occlusion scene on {:?}",
        fixture.adapter.get_info()
    );
    let blocks = Blocks::find(&records);
    let meshes = meshes(&assets, blocks.air, &scene(&blocks));
    let (mut app, camera) = app(&fixture, assets);
    let installed = app
        .sub_app(RenderApp)
        .world()
        .contains_resource::<super::GpuCullSupport>();
    if !installed
        || !app
            .sub_app(RenderApp)
            .world()
            .resource::<super::GpuCullSupport>()
            .0
    {
        eprintln!("skipping transparent occlusion scene: no GPU cull on this adapter");
        return;
    }
    // The cull's kernels compile on a worker; frames advance until it publishes them.
    for _ in 0..600 {
        if app
            .sub_app(RenderApp)
            .world()
            .contains_resource::<GpuCull>()
        {
            break;
        }
        frames(&mut app, 1);
    }
    {
        let mut queue = app
            .world_mut()
            .resource_mut::<render_scene::ChunkRenderQueue>();
        for (key, mesh) in meshes {
            queue
                .try_insert(key, mesh, render_scene::ChunkUploadPriority::new(0.0))
                .unwrap();
        }
    }
    frames(&mut app, SETTLE_FRAMES * 2);
    // Each step moves the eye, so the cull history and the last pyramid are stale at first.
    let views = [
        (
            "front",
            Vec3::new(24.0, 8.0, 30.0),
            Vec3::new(24.0, 4.0, -16.0),
        ),
        (
            "strafe",
            Vec3::new(27.0, 9.0, 28.0),
            Vec3::new(20.0, 4.0, -16.0),
        ),
        (
            "edge",
            Vec3::new(30.0, 10.0, 26.0),
            Vec3::new(44.0, 3.0, -12.0),
        ),
        (
            "glass",
            Vec3::new(24.0, 4.0, 29.5),
            Vec3::new(24.0, 2.0, 16.0),
        ),
        (
            "over",
            Vec3::new(24.0, 30.0, 40.0),
            Vec3::new(24.0, 0.0, -20.0),
        ),
        (
            "underwater",
            Vec3::new(8.25, 4.5, 40.75),
            Vec3::new(8.0, 4.0, 0.0),
        ),
    ];
    let mut culled_views = 0;
    for (name, eye, target) in views {
        *app.world_mut().get_mut::<Transform>(camera).unwrap() =
            Transform::from_translation(eye).looking_at(target, Vec3::Y);
        let set = |app: &mut App, enabled: bool| {
            app.sub_app_mut(RenderApp)
                .world_mut()
                .resource_mut::<GpuCull>()
                .transparent
                .set_enabled(enabled);
        };
        set(&mut app, true);
        frames(&mut app, SETTLE_FRAMES);
        let before = cull(&app).transparent.encodes();
        // The readback may trail by a frame, so both captured candidates must be culled.
        frames(&mut app, 2);
        let culled = cull(&app).transparent.encodes() - before == 2;
        let on = app.world().resource::<Capture>().0.clone();
        let hidden = hidden_slots(&app);
        let drawn = app
            .sub_app(RenderApp)
            .world()
            .resource::<TransparentSubmissions>()
            .last_frame();
        set(&mut app, false);
        frames(&mut app, SETTLE_FRAMES);
        let off = app.world().resource::<Capture>().0.clone();
        save(&format!("transparent-occlusion-{name}-on"), &on);
        save(&format!("transparent-occlusion-{name}-off"), &off);
        let differing = mismatches(&on, &off);
        eprintln!(
            "{name}: culled {culled}, hidden slots {hidden}, drawn {drawn:?}, differing pixels \
             {differing}"
        );
        assert_eq!(on.len(), (SIZE[0] * SIZE[1] * 4) as usize);
        assert_eq!(differing, 0, "{name}: occlusion changed pixels");
        assert!(drawn.water_faces > 0, "{name}: the view must draw water");
        culled_views += usize::from(culled && hidden > 0);
    }
    assert!(
        culled_views > 0,
        "some captured view must draw with hidden transparent slots"
    );
}
