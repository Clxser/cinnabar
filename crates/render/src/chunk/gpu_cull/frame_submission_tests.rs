//! Command buffers and queue submissions of a production-like frame, and screenshots and GPU
//! readbacks that complete inside the render graph's single submission.

use super::app_tests::{
    KEYS, camera_transform, chunk_app_with, frame, insert_meshes, noop_render_plugin, render_plugin,
};
use super::*;
use bevy::{
    camera::RenderTarget,
    prelude::{On, UVec2},
    render::{
        RenderPlugin,
        gpu_readback::{Readback, ReadbackComplete},
        renderer::{PendingCommandBuffers, RenderGraph, RenderGraphSystems, render_system},
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
};

/// Frames measured once the fixture's pipelines and uploads have settled.
const MEASURED_FRAMES: usize = 4;

/// Frames a screenshot and a readback may take to reach the main world.
const CAPTURE_FRAMES: usize = 8;

/// What each rendered frame queued and submitted.
///
/// Queue submission indices count every submission and also every buffer map request, so the
/// graph's own submission is told apart from anything else by where the indices are consumed.
#[derive(Resource, Default)]
struct SubmissionCounts {
    /// Command buffers pending once every graph system before the submission has recorded.
    graph_buffers: Vec<usize>,
    /// Submission indices consumed by the graph's `Submit` stage.
    submit_stage: Vec<u64>,
    /// Submission indices `render_system` consumed outside the graph's `Submit` stage.
    outside_submit_stage: Vec<u64>,
    /// Markers at the start of `render_system`, and the start and end of the `Submit` stage.
    marks: [u64; 3],
}

/// Reserves a queue submission index; consecutive markers bracket what ran between them.
fn marker(queue: &RenderQueue) -> u64 {
    crate::device_poll::submission_marker(queue)
}

fn mark_render_start(queue: Res<RenderQueue>, mut counts: ResMut<SubmissionCounts>) {
    counts.marks[0] = marker(&queue);
}

fn mark_submit_start(
    queue: Res<RenderQueue>,
    pending: Res<PendingCommandBuffers>,
    mut counts: ResMut<SubmissionCounts>,
) {
    counts.graph_buffers.push(pending.len());
    counts.marks[1] = marker(&queue);
}

fn mark_submit_end(queue: Res<RenderQueue>, mut counts: ResMut<SubmissionCounts>) {
    counts.marks[2] = marker(&queue);
    let consumed = counts.marks[2] - counts.marks[1] - 1;
    counts.submit_stage.push(consumed);
}

fn mark_render_end(queue: Res<RenderQueue>, mut counts: ResMut<SubmissionCounts>) {
    let end = marker(&queue);
    let [start, submit_start, submit_end] = counts.marks;
    let outside = (submit_start - start - 1) + (end - submit_end - 1);
    counts.outside_submit_stage.push(outside);
}

fn install_counters(app: &mut App) {
    app.sub_app_mut(RenderApp)
        .init_resource::<SubmissionCounts>()
        .add_systems(
            Render,
            (
                mark_render_start
                    .in_set(RenderSystems::Render)
                    .before(render_system),
                mark_render_end
                    .in_set(RenderSystems::Render)
                    .after(render_system)
                    .before(crate::device_poll::FrameSubmissions),
            ),
        )
        .add_systems(
            RenderGraph,
            (
                mark_submit_start
                    .after(RenderGraphSystems::Render)
                    .after(crate::gpu_timing::encode_readback)
                    .before(RenderGraphSystems::Submit),
                mark_submit_end
                    .in_set(RenderGraphSystems::Finish)
                    .after(RenderGraphSystems::Submit),
            ),
        );
}

/// The terrain fixture plus the production world, actor, hand, UI and GPU timing passes.
fn production_app(render: RenderPlugin) -> (App, Entity) {
    let (mut app, camera) = chunk_app_with(render, Msaa::Sample4, camera_transform(), |app| {
        // Readbacks and screenshots copy out of the camera's target.
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        for (_, image) in images.iter_mut() {
            image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        }
        app.insert_resource(render_scene::RuntimeStageProfiler::new(false));
        app.add_plugins((
            crate::AtmospherePlugin,
            crate::ActorRenderPlugin,
            crate::HandRigRenderPlugin,
            crate::ViewmodelRenderPlugin,
            crate::UiRenderPlugin,
            crate::ParticleRenderPlugin,
            crate::GpuTimingPlugin,
        ));
        install_counters(app);
    });
    insert_meshes(&mut app, &KEYS);
    for _ in 0..8 {
        frame(&mut app);
    }
    reset_counts(&mut app);
    (app, camera)
}

fn reset_counts(app: &mut App) {
    *app.sub_app_mut(RenderApp)
        .world_mut()
        .resource_mut::<SubmissionCounts>() = SubmissionCounts::default();
}

fn counts(app: &App) -> &SubmissionCounts {
    app.sub_app(RenderApp)
        .world()
        .resource::<SubmissionCounts>()
}

fn production_features() -> WgpuFeatures {
    WgpuFeatures::TIMESTAMP_QUERY
        | WgpuFeatures::INDIRECT_FIRST_INSTANCE
        | WgpuFeatures::MULTI_DRAW_INDIRECT_COUNT
}

/// Serial render systems share the frame's command encoder, and `render_system` submits nothing
/// besides the graph's submission: no separate encoder for screenshots and readbacks.
#[test]
fn production_frames_record_one_command_buffer_and_submit_only_the_graph() {
    let (mut app, _) = production_app(noop_render_plugin(production_features()));
    for _ in 0..MEASURED_FRAMES {
        frame(&mut app);
    }
    let counts = counts(&app);
    eprintln!(
        "graph command buffers per frame: {:?}; submission indices in the graph's submit stage: \
         {:?}, elsewhere in render_system: {:?}",
        counts.graph_buffers, counts.submit_stage, counts.outside_submit_stage
    );
    assert_eq!(counts.graph_buffers, [1; MEASURED_FRAMES]);
    assert_eq!(counts.outside_submit_stage, [0; MEASURED_FRAMES]);
}

#[derive(Resource, Default)]
struct Captures {
    screenshot: Option<UVec2>,
    readback: Option<usize>,
}

/// Captures the camera's target with a screenshot and a GPU readback.
fn assert_captures_complete(render: RenderPlugin, label: &str) {
    let (mut app, camera) = production_app(render);
    let RenderTarget::Image(target) = app.world().get::<RenderTarget>(camera).unwrap().clone()
    else {
        panic!("fixture must use an image target");
    };
    app.init_resource::<Captures>();
    app.world_mut()
        .spawn(Screenshot::image(target.handle.clone()))
        .observe(
            |captured: On<ScreenshotCaptured>, mut captures: ResMut<Captures>| {
                captures.screenshot = Some(captured.image.size());
            },
        );
    let readback = app
        .world_mut()
        .spawn(Readback::texture(target.handle))
        .observe(
            |event: On<ReadbackComplete>, mut captures: ResMut<Captures>| {
                captures.readback = Some(event.data.len());
            },
        )
        .id();
    let mut frames = 0;
    while frames < CAPTURE_FRAMES {
        frame(&mut app);
        frames += 1;
        let captures = app.world().resource::<Captures>();
        if captures.screenshot.is_some() && captures.readback.is_some() {
            break;
        }
    }
    let capture_counts = counts(&app);
    eprintln!(
        "{label}: captures completed after {frames} frames; graph command buffers per frame: \
         {:?}; submission indices in the graph's submit stage: {:?}, elsewhere in \
         render_system: {:?}",
        capture_counts.graph_buffers,
        capture_counts.submit_stage,
        capture_counts.outside_submit_stage
    );
    let captures = app.world().resource::<Captures>();
    assert_eq!(captures.screenshot, Some(UVec2::new(64, 48)), "{label}");
    assert_eq!(captures.readback, Some(64 * 48 * 4), "{label}");
    assert!(
        capture_counts
            .graph_buffers
            .iter()
            .all(|&buffers| buffers == 1),
        "{label}: the graph still records into one command buffer"
    );
    // A `Readback` repeats every frame until removed.
    app.world_mut().despawn(readback);
    reset_counts(&mut app);
    frame(&mut app);
    frame(&mut app);
    let after = &counts(&app).outside_submit_stage;
    eprintln!("{label}: after the captures, elsewhere in render_system: {after:?}");
    assert_eq!(
        after.last(),
        Some(&0),
        "{label}: once captured, render_system submits only the graph again"
    );
}

#[test]
fn screenshots_and_readbacks_complete_inside_the_graph_submission() {
    assert_captures_complete(noop_render_plugin(production_features()), "noop");
    if let Some(render) = render_plugin(wgpu::Backends::PRIMARY, production_features()) {
        assert_captures_complete(render, "native");
    } else {
        eprintln!("skipping native screenshot and readback: missing native GPU adapter fixture");
    }
}
