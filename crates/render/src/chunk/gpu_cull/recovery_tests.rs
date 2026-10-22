//! Terrain publication that runs out of GPU memory at any growth site converges to the same
//! state as a publication that never failed.

use std::{cell::Cell, rc::Rc};

use bevy::{
    ecs::schedule::{ScheduleLabel, SingleThreadedExecutor},
    render::{ExtractSchedule, Render},
};

use super::app_tests::{
    KEYS, camera_transform, chunk_app, frame, insert_meshes, noop_render_plugin,
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
    Published {
        allocations,
        admitted,
        batches,
        gpu_culled: culled.is_some(),
        culled_slots: world.resource::<GpuCull>().slot_count(),
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

fn terrain_app(culling: bool) -> App {
    let (mut app, _) = chunk_app(
        noop_render_plugin(WgpuFeatures::INDIRECT_FIRST_INSTANCE),
        Msaa::Off,
        camera_transform(),
    );
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
    insert_meshes(&mut app, &KEYS);
    app
}

/// Injects out-of-memory at one growth site for the first frames, then lets every frame
/// succeed; the result must equal a publication that never failed.
#[test]
fn every_growth_site_recovers_to_the_fresh_publication() {
    const FAILING_FRAMES: usize = 2;
    const SETTLE_FRAMES: usize = 6;
    for (site, culling) in [
        ("packed chunk quads", false),
        ("packed chunk geometry streams", false),
        ("packed chunk origins", false),
        ("terrain identity vertex offsets", false),
        ("packed chunk indirect commands", false),
        ("terrain cull storage", true),
    ] {
        let mut fresh = terrain_app(culling);
        for _ in 0..SETTLE_FRAMES {
            frame(&mut fresh);
        }
        let mut recovering = terrain_app(culling);
        let failures = Rc::new(Cell::new(0));
        {
            let failures = Rc::clone(&failures);
            let _failure = crate::gpu_allocation::inject(move |label, _| {
                let fail = label == site;
                failures.set(failures.get() + usize::from(fail));
                fail
            });
            for _ in 0..FAILING_FRAMES {
                frame(&mut recovering);
            }
        }
        assert!(failures.get() > 0, "{site} was never allocated");
        // Witnesses describe only submitted commands, even while publication is failing.
        let failing = published(&mut recovering);
        assert!(
            failing
                .batches
                .iter()
                .all(|(commands, drawn)| (*commands == 0) == drawn.is_empty()),
            "{site}: {failing:?}"
        );
        for _ in 0..SETTLE_FRAMES {
            frame(&mut recovering);
        }
        assert_eq!(published(&mut recovering), published(&mut fresh), "{site}");
        assert!(
            !recovering
                .sub_app(RenderApp)
                .world()
                .resource::<ChunkGpuArena>()
                .indirect_publication_stale,
            "{site}"
        );
    }
}
