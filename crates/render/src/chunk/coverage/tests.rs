use super::*;
use bevy::prelude::MinimalPlugins;
use render_scene::{ChunkRenderInstance, ChunkRenderQueue, ChunkUploadPriority, ChunkUploadToken};

#[test]
fn cancelled_queued_withdrawal_keeps_the_existing_near_instance() {
    let key = SubChunkKey::new(0, 5, 0, 0);
    let assets = assets::RuntimeAssets::diagnostic();
    let source = world::SubChunk::decode(&[9, 1, 0, 1, 2], &world::RawBlockIds { air: 0 });
    let mesh = meshing::mesh_sub_chunk(
        &meshing::BlockClassifier::new(0),
        &assets,
        assets::NetworkIdMode::Sequential,
        &meshing::Neighbourhood::empty(),
        &source,
    );
    assert!(!mesh.is_empty());
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(ChunkRenderPlugin::default())
        .insert_resource(FarTerrainScene {
            target: Some(Arc::new(far_terrain::TerrainTarget {
                centre: key.chunk(),
                horizon: 32,
                near: std::sync::Arc::new(HashMap::from([(
                    key.chunk(),
                    HashMap::from([(key, 3)]),
                )])),
                retained: std::sync::Arc::new(HashSet::from([key])),
                empty_far: std::sync::Arc::new(HashSet::new()),
            })),
            ..Default::default()
        });
    let since = Instant::now();
    app.world_mut()
        .resource_mut::<ChunkRenderQueue>()
        .try_update_tracked(
            key,
            mesh,
            ChunkUploadPriority::new(0.0),
            ChunkUploadToken {
                generation: 1,
                dirty_since: since,
            },
        )
        .unwrap();
    app.update();
    app.world_mut()
        .resource_mut::<ChunkRenderQueue>()
        .try_remove_tracked(
            key,
            ChunkUploadPriority::new(0.0),
            ChunkUploadToken {
                generation: 2,
                dirty_since: since,
            },
        )
        .unwrap();
    app.update();
    let world = app.world_mut();
    let mut query = world.query::<&ChunkRenderInstance>();
    let remaining = query
        .single(world)
        .expect("cancelled withdrawal retains the old cover");
    assert_eq!((remaining.key, remaining.generation), (key, 1));
    assert_eq!(world.resource::<ChunkRenderQueue>().retained_len(), 0);
}

/// Near sections uploaded in a frame whose far mask cannot be uploaded are admitted only where
/// the mask still on the GPU hides far terrain, so partial near and far never draw together.
#[test]
fn a_failed_mask_upload_admits_against_the_mask_on_the_gpu() {
    use bevy::ecs::system::RunSystemOnce;
    let (device, queue) = wgpu::Device::noop(&wgpu::DeviceDescriptor::default());
    let (device, queue) = (RenderDevice::from(device), RenderQueue::new(queue));
    let key = SubChunkKey::new(0, 0, 0, 0);
    let mut far = crate::far_terrain::FarTerrainGpu::new(&device);
    let tile = Arc::new(far_terrain::TileMesh {
        coverage: far_terrain::TileCoverage::Complete,
        key: far_terrain::TileKey {
            level: 0,
            x: 0,
            z: 0,
        },
        revision: 1,
        quads: vec![[0, 64]].into_boxed_slice(),
        colours: vec![0; far_terrain::config::TILE_CELLS * far_terrain::config::TILE_CELLS * 4]
            .into_boxed_slice(),
        min_y: 0,
        max_y: 64,
    });
    assert!(far.apply_tiles(&device, &queue, &[tile]));
    let mut allocation = crate::chunk::tests::retirement_test_allocation();
    allocation.gpu.key = key;
    allocation.gpu.generation = 3;
    let mut app = App::new();
    let entity = app.world_mut().spawn(allocation.gpu.clone()).id();
    let mut arena = ChunkGpuArena::new(&device);
    arena.allocations.insert(entity, allocation);
    app.insert_resource(arena)
        .insert_resource(far)
        .insert_resource(device)
        .insert_resource(queue)
        .init_resource::<TerrainCoverage>()
        .insert_resource(FarTerrainScene {
            target: Some(Arc::new(far_terrain::TerrainTarget {
                centre: key.chunk(),
                horizon: 32,
                near: Arc::new(HashMap::from([(key.chunk(), HashMap::from([(key, 3)]))])),
                retained: Arc::new(HashSet::from([key])),
                empty_far: Arc::new(HashSet::new()),
            })),
            ..Default::default()
        });
    {
        let _failure = crate::gpu_allocation::inject(|label, _| label.starts_with("far terrain"));
        app.world_mut()
            .run_system_once(publish_terrain_coverage)
            .unwrap();
    }
    assert!(
        app.world().get::<GpuChunkAllocation>(entity).is_none(),
        "far terrain still draws the column, so its near section stays hidden"
    );
    app.world_mut()
        .run_system_once(publish_terrain_coverage)
        .unwrap();
    assert!(
        app.world().get::<GpuChunkAllocation>(entity).is_some(),
        "the retried mask hides far terrain and admits the section"
    );
}
