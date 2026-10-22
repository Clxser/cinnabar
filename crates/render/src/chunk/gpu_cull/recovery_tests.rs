//! Terrain publication that runs out of GPU memory at any growth site converges to the same
//! state as a publication that never failed.

use std::{cell::Cell, rc::Rc};

use bevy::{
    ecs::schedule::{ScheduleLabel, SingleThreadedExecutor},
    render::{ExtractSchedule, Render},
};

use super::app_tests::{
    KEYS, camera_transform, chunk_app_with, frame, mesh, noop_render_plugin, render_plugin,
};
use super::*;

/// What terrain publication drew and reported: residents, admitted draws, the commands each
/// submitted indirect batch draws with the allocations it witnesses, and the culled slots.
#[derive(Debug, PartialEq, Eq)]
struct Published {
    allocations: Vec<(SubChunkKey, u64)>,
    admitted: Vec<SubChunkKey>,
    batches: Vec<(u32, Vec<(SubChunkKey, u64)>)>,
    gpu_culled: bool,
    culled_slots: u32,
    geometry: Vec<Vec<u8>>,
    indirect: Vec<u8>,
    far: Vec<Vec<u8>>,
}

/// The view the last frame culled on the GPU; its CPU batches are retained but not submitted.
#[derive(Resource, Default)]
struct CulledView(Option<Entity>);

fn published(app: &mut App) -> Published {
    let world = app.sub_app_mut(RenderApp).world_mut();
    let mut admitted: Vec<_> = world
        .query::<&GpuChunkAllocation>()
        .iter(world)
        .map(|allocation| allocation.key)
        .collect();
    admitted.sort_unstable();
    let mut allocations: Vec<_> = world
        .resource::<ChunkGpuArena>()
        .allocations
        .values()
        .map(|allocation| (allocation.gpu.key, allocation.generation))
        .collect();
    allocations.sort_unstable();
    let witnessed = |batch: &ChunkIndirectBatch| {
        let mut drawn: Vec<_> = batch
            .drawn_allocations
            .iter()
            .map(|(_, identity)| (identity.key, identity.generation))
            .collect();
        drawn.sort_unstable();
        (batch.command_count, drawn)
    };
    let culled = world.resource::<CulledView>().0;
    let submitted = |view: &&Entity| Some(**view) != culled;
    let mut batches: Vec<_> = world
        .resource::<crate::chunk::pipeline::solid::ChunkSolidIndirectBatches>()
        .0
        .iter()
        .filter(|(view, _)| submitted(view))
        .map(|(_, batch)| witnessed(&batch.cubes))
        .collect();
    for set in [
        &world.resource::<ChunkIndirectBatches>().0,
        &world.resource::<ChunkModelIndirectBatches>().0,
        &world.resource::<ChunkDepthLiquidIndirectBatches>().0,
    ] {
        batches.extend(
            set.iter()
                .filter(|(view, _)| submitted(view))
                .map(|(_, batch)| witnessed(batch)),
        );
    }
    batches.sort_unstable();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let arena = world.resource::<ChunkGpuArena>();
    let geometry = [
        (&arena.quad_buffer, arena.quad_len, PACKED_QUAD_BYTES),
        (
            &arena.geometry_stream_buffer,
            arena.geometry_stream_len,
            GEOMETRY_STREAM_WORD_BYTES,
        ),
        (&arena.origin_buffer, arena.origin_len, CHUNK_ORIGIN_BYTES),
        (&arena.biome_buffer, arena.biome_len, BIOME_WORD_BYTES),
    ]
    .into_iter()
    .map(|(buffer, len, stride)| {
        crate::render_test_support::read_buffer(device, queue, buffer, 0, len as u64 * stride)
    })
    .collect();
    Published {
        geometry,
        indirect: arena.uploaded_indirect_bytes.clone(),
        far: world
            .resource::<crate::far_terrain::FarTerrainGpu>()
            .publication_snapshot(device, queue),
        allocations,
        admitted,
        batches,
        gpu_culled: culled.is_some(),
        culled_slots: world
            .get_resource::<GpuCull>()
            .map_or(0, GpuCull::slot_count),
    }
}

/// Runs render schedules on this thread, where the allocation fault injector applies.
fn single_threaded(app: &mut App) {
    let render = app.sub_app_mut(RenderApp);
    for label in [Render.intern(), ExtractSchedule.intern()] {
        if let Some(schedule) = render
            .world_mut()
            .resource_mut::<bevy::ecs::schedule::Schedules>()
            .get_mut(label)
        {
            schedule.set_executor(SingleThreadedExecutor::new());
        }
    }
}

/// Builds a real terrain publication with non-fallback biomes, far tiles, and either draw path.
fn terrain_app(culling: bool, direct: bool, hardware: bool) -> Option<App> {
    let features = if direct {
        WgpuFeatures::empty()
    } else {
        WgpuFeatures::INDIRECT_FIRST_INSTANCE
    };
    let render = if hardware {
        render_plugin(wgpu::Backends::PRIMARY, features)?
    } else {
        noop_render_plugin(features)
    };
    let (mut app, _) = chunk_app_with(render, Msaa::Off, camera_transform(), |app| {
        app.add_plugins(crate::far_terrain::FarTerrainRenderPlugin);
    });
    single_threaded(&mut app);
    app.sub_app_mut(RenderApp)
        .init_resource::<CulledView>()
        .add_systems(
            Render,
            (|frame: Res<GpuCullFrame>, mut culled: ResMut<CulledView>| {
                culled.0 = frame.view.map(|view| view.entity);
            })
            .in_set(RenderSystems::Render),
        );
    app.world_mut()
        .resource_mut::<render_scene::ChunkOcclusionCulling>()
        .0 = culling;
    let biomes = world::DecodedBiomeColumn::decode(
        -4,
        1,
        &[1, 84],
        &world::RawBiomeIds { default_biome: 0 },
    );
    let biome = meshing::PackedBiomeRecord::from_storage(&biomes.storage(-4).unwrap(), |id| id + 1);
    for key in KEYS {
        let mut mesh = mesh();
        mesh.biome = biome.clone();
        app.world_mut()
            .resource_mut::<render_scene::ChunkRenderQueue>()
            .try_insert(key, mesh, render_scene::ChunkUploadPriority::urgent())
            .unwrap();
    }
    let mut scene = app
        .world_mut()
        .resource_mut::<render_scene::FarTerrainScene>();
    scene.near = Arc::new(far_terrain::NearMask::from_columns([0, 0], 32, []));
    scene.tiles = (0..2)
        .map(|index| {
            Arc::new(far_terrain::TileMesh {
                coverage: far_terrain::TileCoverage::Complete,
                key: far_terrain::TileKey {
                    level: 0,
                    x: index + 4,
                    z: 0,
                },
                revision: index as u64 + 1,
                quads: vec![[index as u32, 64]; 2].into_boxed_slice(),
                colours: vec![
                    index as u8;
                    far_terrain::config::TILE_CELLS * far_terrain::config::TILE_CELLS * 4
                ]
                .into_boxed_slice(),
                min_y: 0,
                max_y: 64,
            })
        })
        .collect::<Vec<_>>()
        .into();
    Some(app)
}

/// Every checked terrain growth site retries a complete publication on the very next frame.
#[test]
fn every_growth_site_recovers_to_the_fresh_publication() {
    recovery_table(false);
}

/// Hardware readback checks geometry bytes as well as the validating NOOP schedule checks.
#[test]
fn terrain_recovery_matches_fresh_gpu_bytes() {
    recovery_table(true);
}

/// Exercises the same error-scope boundary as production, including late staging growth.
fn recovery_table(hardware: bool) {
    for (site, culling, direct) in [
        ("packed chunk quads", false, false),
        ("packed chunk geometry streams", false, false),
        ("packed chunk origins", false, false),
        ("packed chunk biome records", false, false),
        ("terrain identity vertex offsets", false, false),
        ("packed chunk indirect commands", false, false),
        ("terrain cull storage", true, false),
        ("terrain occlusion storage", true, true),
        ("far terrain quads", false, false),
        ("far terrain tile slots", false, false),
        ("far terrain near columns", false, false),
        ("retained terrain upload staging", false, false),
    ] {
        let Some(mut fresh) = terrain_app(culling, direct, hardware) else {
            eprintln!("missing-fixture: terrain recovery GPU adapter");
            return;
        };
        let mut recovering = terrain_app(culling, direct, hardware).unwrap();
        for _ in 0..4 {
            frame(&mut fresh);
        }
        // Metal deliberately uses direct submission, so indirect/cull storage has no growth.
        if hardware
            && !direct
            && fresh
                .sub_app(RenderApp)
                .world()
                .get_resource::<GpuCull>()
                .is_none()
            && (site == "terrain cull storage" || site == "packed chunk indirect commands")
        {
            continue;
        }
        let failures = Rc::new(Cell::new(0));
        {
            let failures = Rc::clone(&failures);
            let _failure = crate::gpu_allocation::inject(move |label, _| {
                let fail = label == site;
                failures.set(failures.get() + usize::from(fail));
                fail
            });
            for _ in 0..4 {
                frame(&mut recovering);
                if site == "retained terrain upload staging" {
                    recovering
                        .sub_app(RenderApp)
                        .world()
                        .resource::<crate::upload_staging::TerrainUploadStaging>()
                        .request_test_growth();
                }
            }
            assert!(failures.get() > 0, "{site} was never allocated");
        }
        frame(&mut recovering);
        assert_eq!(published(&mut recovering), published(&mut fresh), "{site}");
        assert_eq!(
            recovering
                .sub_app(RenderApp)
                .world()
                .resource::<ChunkGpuUploadStats>()
                .chunk_updates,
            KEYS.len(),
            "{site} did not republish CPU geometry"
        );
        // Once rebuilt, another frame retains the exact publication without replaying it.
        frame(&mut recovering);
        assert_eq!(
            published(&mut recovering),
            published(&mut fresh),
            "{site} was not idempotent"
        );
        assert_eq!(
            recovering
                .sub_app(RenderApp)
                .world()
                .resource::<ChunkGpuUploadStats>()
                .chunk_updates,
            0,
            "{site}"
        );
    }
}
