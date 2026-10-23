//! Terrain publication that runs out of GPU memory at any growth site converges to the same
//! state as a publication that never failed.

use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

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
    geometry: Vec<[u8; 32]>,
    indirect: Vec<u8>,
    far: Vec<[u8; 32]>,
    cull: Vec<[u8; 32]>,
    direct: Vec<[u8; 32]>,
    units: Vec<[u8; 32]>,
}

/// The view the last frame culled on the GPU; its CPU batches are retained but not submitted.
#[derive(Resource, Default)]
struct CulledView(Option<Entity>);

/// Reads the logical publication and the geometry bytes referenced by its GPU draws.
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
        Sha256::digest(crate::render_test_support::read_buffer(
            device,
            queue,
            buffer,
            0,
            len as u64 * stride,
        ))
        .into()
    })
    .collect();
    Published {
        geometry,
        indirect: crate::render_test_support::read_buffer(
            device,
            queue,
            &arena.indirect_buffer,
            0,
            arena.uploaded_indirect_bytes.len() as u64,
        ),
        cull: world
            .get_resource::<GpuCull>()
            .map_or_else(Vec::new, |cull| cull.publication_snapshot(device, queue))
            .into_iter()
            .map(|bytes| Sha256::digest(bytes).into())
            .collect(),
        direct: world
            .get_resource::<direct::DirectOcclusion>()
            .map_or_else(Vec::new, |direct| {
                direct.publication_snapshot(device, queue)
            })
            .into_iter()
            .map(|bytes| Sha256::digest(bytes).into())
            .collect(),
        units: world
            .resource::<crate::chunk::pipeline::units::TerrainUnitBatches>()
            .publication_snapshot(device, queue)
            .into_iter()
            .map(|bytes| Sha256::digest(bytes).into())
            .collect(),
        far: world
            .resource::<crate::far_terrain::FarTerrainGpu>()
            .publication_snapshot(device, queue)
            .into_iter()
            .map(|bytes| Sha256::digest(bytes).into())
            .collect(),
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
    let config = render_api::PublicationServiceConfig::DEFAULT;
    let allowance = render_api::PublicationAllowance::new(config);
    allowance.begin_frame(
        1,
        KEYS.len(),
        config.maximum_frame_bytes,
        0,
        config.maximum_frame_items,
    );
    for key in KEYS {
        let mesh = mesh();
        let bytes = render_scene::ChunkRenderQueue::upload_byte_len(&mesh, &biome);
        let permit = allowance.try_admit_payload(bytes).unwrap();
        app.world_mut()
            .resource_mut::<render_scene::ChunkRenderQueue>()
            .try_update_tracked_with_biome_identity_permitted(
                key,
                mesh,
                biome.clone(),
                ChunkBiomeTintIdentity::default(),
                render_scene::ChunkUploadPriority::urgent(),
                render_scene::ChunkUploadToken {
                    generation: 1,
                    dirty_since: Instant::now(),
                },
                permit,
            )
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

/// Required terrain growth retries a complete publication on the very next frame.
#[test]
fn every_growth_site_recovers_to_the_fresh_publication() {
    recovery_table(false);
}

/// Hardware readback checks geometry bytes as well as the validating NOOP schedule checks.
#[test]
fn terrain_recovery_matches_fresh_gpu_bytes() {
    recovery_table(true);
}

/// Exercises required publication allocations and optional late staging growth.
fn recovery_table(hardware: bool) {
    let sites = [
        ("packed chunk quads", false, false),
        ("packed chunk geometry streams", false, false),
        ("packed chunk origins", false, false),
        ("packed chunk biome records", false, false),
        ("terrain identity vertex offsets", false, false),
        ("packed chunk indirect commands", false, false),
        ("terrain cull storage", true, false),
        ("terrain occlusion storage", true, true),
        ("far terrain quads", false, false),
        ("far terrain near columns", false, false),
        ("retained terrain upload staging", false, false),
        ("double-buffered transparent draw refs", false, false),
        ("terrain arena upload", false, false),
        ("terrain recovery biome upload", false, false),
        ("terrain indirect upload", false, false),
        ("far terrain tile upload", false, false),
        ("far terrain mask upload", false, false),
        ("indexed terrain unit page", false, true),
    ];
    let observed = Rc::new(RefCell::new(HashSet::new()));
    let mut covered = HashSet::new();
    for &(site, culling, direct) in &sites {
        let Some(mut fresh) = terrain_app(culling, direct, hardware) else {
            eprintln!("missing-fixture: terrain recovery GPU adapter");
            return;
        };
        let mut recovering = terrain_app(culling, direct, hardware).unwrap();
        if site == "far terrain quads" {
            // One draw requires growth; room for further tiles is optional and may back off.
            for app in [&mut fresh, &mut recovering] {
                let mut scene = app
                    .world_mut()
                    .resource_mut::<render_scene::FarTerrainScene>();
                scene.tiles = Arc::from(&scene.tiles[..1]);
            }
        }
        if site == "terrain recovery biome upload" {
            for _ in 0..4 {
                frame(&mut recovering);
            }
            fail_transparent_growth(&mut recovering);
        }
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
            && (site == "terrain cull storage"
                || site == "packed chunk indirect commands"
                || site == "terrain indirect upload")
        {
            continue;
        }
        let failures = Rc::new(Cell::new(0));
        {
            let injected = Rc::clone(&failures);
            let observed = Rc::clone(&observed);
            let _failure = crate::gpu_allocation::inject_with_owner(move |label, _, terrain| {
                if terrain || label == site {
                    observed.borrow_mut().insert(label.to_owned());
                }
                let fail = label == site;
                injected.set(injected.get() + usize::from(fail));
                fail
            });
            for _ in 0..4 {
                frame(&mut recovering);
                if site == "double-buffered transparent draw refs" {
                    let world = recovering.sub_app_mut(RenderApp).world_mut();
                    let device = world.resource::<RenderDevice>().clone();
                    let queue = world.resource::<RenderQueue>().clone();
                    let needed = world.resource::<ChunkGpuArena>().transparent_slot_refs + 1;
                    ensure_transparent_ref_capacity(
                        &mut world.resource_mut::<ChunkGpuArena>(),
                        &device,
                        &queue,
                        needed,
                        &TransparentSortState::with_upload_cap(usize::MAX),
                    );
                }
                if site == "retained terrain upload staging" {
                    assert!(
                        recovering
                            .sub_app(RenderApp)
                            .world()
                            .resource::<ChunkGpuArena>()
                            .bind_group
                            .is_some()
                    );
                    recovering
                        .sub_app(RenderApp)
                        .world()
                        .resource::<crate::upload_staging::TerrainUploadStaging>()
                        .request_test_growth();
                }
            }
            assert!(failures.get() > 0, "{site} was never allocated");
        }
        covered.insert(site.to_owned());
        frame(&mut recovering);
        assert_eq!(published(&mut recovering), published(&mut fresh), "{site}");
        assert_eq!(
            recovering
                .sub_app(RenderApp)
                .world()
                .resource::<ChunkGpuUploadStats>()
                .chunk_updates,
            if site == "retained terrain upload staging" {
                0
            } else {
                KEYS.len()
            },
            "{site} did not republish CPU geometry"
        );
        assert_eq!(
            recovering
                .world()
                .resource::<render_scene::ChunkUploadAcknowledgements>()
                .drain()
                .len(),
            KEYS.len(),
            "{site} acknowledged a replay twice"
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
    assert!(
        observed.borrow().iter().all(|site| covered.contains(site)),
        "allocation sites missing a failure case: {:?}",
        observed.borrow()
    );
}

/// Recovery upload failures suppress terrain until a complete retry succeeds.
#[test]
fn recovery_biome_upload_without_memory_retries() {
    let mut app = terrain_app(false, false, false).unwrap();
    for _ in 0..4 {
        frame(&mut app);
    }
    let expected = published(&mut app);
    fail_transparent_growth(&mut app);
    let failures = Rc::new(Cell::new(0));
    {
        let observed = Rc::clone(&failures);
        let _failure = crate::gpu_allocation::inject(move |label, _| {
            let failed = label == "terrain recovery biome upload";
            observed.set(observed.get() + usize::from(failed));
            failed
        });
        for _ in 0..2 {
            frame(&mut app);
            assert!(
                app.sub_app(RenderApp)
                    .world()
                    .resource::<ChunkGpuArena>()
                    .bind_group
                    .is_none()
            );
        }
    }
    assert_eq!(failures.get(), 2);
    frame(&mut app);
    assert!(
        app.sub_app(RenderApp)
            .world()
            .resource::<ChunkGpuArena>()
            .bind_group
            .is_some()
    );
    assert_eq!(published(&mut app), expected);
}

/// A failed publication withdraws shader-pack expansion and camera and shadow draws together.
#[cfg(feature = "shaderpacks")]
#[test]
fn failed_publication_withdraws_shaderpack_work() {
    use crate::shaderpack::geometry::ShaderpackGeometry;
    let mut app = terrain_app(false, false, false).unwrap();
    for _ in 0..4 {
        frame(&mut app);
    }
    let world = app.sub_app_mut(RenderApp).world_mut();
    let geometry = ShaderpackGeometry::pending_fixture(world.resource::<RenderDevice>());
    assert_eq!(geometry.pending_work(), [1, 1, 1]);
    world.insert_resource(geometry);
    fail_transparent_growth(&mut app);
    let world = app.sub_app_mut(RenderApp).world_mut();
    crate::chunk::recovery::finish_publication(world);
    assert_eq!(
        world.resource::<ShaderpackGeometry>().pending_work(),
        [0; 3]
    );
    assert!(!world.resource::<ShaderpackGeometry>().all_sections_fit());
}

/// A bounded replay stays hidden until all previously published permits have been replayed.
#[test]
fn recovery_retains_pending_replays_without_acknowledging_their_permits_again() {
    check_bounded_replay(render_scene::ChunkUploadBudget::new(1, u64::MAX));
}

/// Previously admitted terrain makes progress even when fresh uploads have no allowance.
#[test]
fn recovery_with_zero_upload_budget_restores_publication() {
    check_bounded_replay(render_scene::ChunkUploadBudget::new(0, 0));
}

/// Recovery admits one payload at a time when the byte allowance cannot hold a replay.
#[test]
fn recovery_with_zero_byte_budget_restores_publication() {
    check_bounded_replay(render_scene::ChunkUploadBudget::new(usize::MAX, 0));
}

/// Replays two resident meshes across frames without spending their publication permits again.
fn check_bounded_replay(budget: render_scene::ChunkUploadBudget) {
    let mut app = terrain_app(false, false, false).unwrap();
    for _ in 0..4 {
        frame(&mut app);
    }
    let expected = published(&mut app);
    assert_eq!(
        app.world()
            .resource::<render_scene::ChunkUploadAcknowledgements>()
            .drain()
            .len(),
        KEYS.len()
    );
    fail_transparent_growth(&mut app);
    app.sub_app_mut(RenderApp)
        .world_mut()
        .insert_resource(budget);
    frame(&mut app);
    let world = app.sub_app(RenderApp).world();
    assert_eq!(world.resource::<ChunkGpuUploadStats>().chunk_updates, 1);
    assert_eq!(
        world
            .resource::<crate::chunk::recovery::TerrainGpuRecovery>()
            .published
            .len(),
        KEYS.len() - 1
    );
    assert!(world.resource::<ChunkGpuArena>().bind_group.is_none());
    frame(&mut app);
    let world = app.sub_app(RenderApp).world();
    assert!(world.resource::<ChunkGpuArena>().bind_group.is_some());
    assert!(
        world
            .resource::<crate::chunk::recovery::TerrainGpuRecovery>()
            .published
            .is_empty()
    );
    assert_eq!(published(&mut app), expected);
    assert!(
        app.world()
            .resource::<render_scene::ChunkUploadAcknowledgements>()
            .drain()
            .is_empty()
    );
}

/// Fails a checked allocation after a complete publication to start the next frame's replay.
fn fail_transparent_growth(app: &mut App) {
    let world = app.sub_app_mut(RenderApp).world_mut();
    let device = world.resource::<RenderDevice>().clone();
    let queue = world.resource::<RenderQueue>().clone();
    let needed = world.resource::<ChunkGpuArena>().transparent_slot_refs + 1;
    {
        let _failure = crate::gpu_allocation::inject(|label, _| {
            label == "double-buffered transparent draw refs"
        });
        ensure_transparent_ref_capacity(
            &mut world.resource_mut::<ChunkGpuArena>(),
            &device,
            &queue,
            needed,
            &TransparentSortState::with_upload_cap(usize::MAX),
        );
    }
}

/// A speculative view-sized allocation cannot suppress a replay that fits retained storage.
#[test]
fn recovery_ignores_capacity_hints_that_exceed_available_memory() {
    let mut app = terrain_app(false, false, false).unwrap();
    for _ in 0..4 {
        frame(&mut app);
    }
    let expected = published(&mut app);
    fail_transparent_growth(&mut app);
    // Include staging and metadata in the memory needed by a complete replay.
    let largest_request = Rc::new(Cell::new(0_u64));
    {
        let largest = Rc::clone(&largest_request);
        let _measure = crate::gpu_allocation::inject(move |_, bytes| {
            largest.set(largest.get().max(bytes));
            false
        });
        frame(&mut app);
    }
    let available_bytes = largest_request.get();
    assert!(available_bytes > 0 && available_bytes < ARENA_MIGRATION_FRAME_BYTES);
    assert_eq!(published(&mut app), expected);
    fail_transparent_growth(&mut app);
    app.world_mut()
        .insert_resource(render_scene::chunk::capacity::ChunkArenaCapacityHint {
            sections: usize::MAX,
        });
    let rejected = Rc::new(RefCell::new(Vec::new()));
    let failures = Rc::clone(&rejected);
    let _failure = crate::gpu_allocation::inject(move |label, bytes| {
        let failed = bytes > available_bytes;
        if failed {
            failures.borrow_mut().push((label.to_owned(), bytes));
        }
        failed
    });
    frame(&mut app);
    assert!(
        app.sub_app(RenderApp)
            .world()
            .resource::<ChunkGpuArena>()
            .bind_group
            .is_some(),
        "replay exceeded its {available_bytes}-byte memory ceiling: {:?}",
        rejected.borrow()
    );
    assert_eq!(published(&mut app), expected);
    frame(&mut app);
    assert_eq!(published(&mut app), expected);
}
