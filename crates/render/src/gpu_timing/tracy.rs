use render_scene::{GpuFrameTimes, RuntimeStage};
use std::sync::LazyLock;
use tracy_client::{Client, PlotName};

static PLOTS: LazyLock<[PlotName; RuntimeStage::GPU.len()]> = LazyLock::new(|| {
    RuntimeStage::GPU
        .map(|stage| PlotName::new_leak(format!("{} elapsed ms (readback)", stage.name())))
});

/// Creates each static plot name once during startup, before frame readbacks.
pub(super) fn initialize() {
    LazyLock::force(&PLOTS);
}

/// Plots delayed pass latencies without inventing GPU execution times on the CPU timeline.
pub(super) fn record(frame: &GpuFrameTimes) {
    let Some(client) = Client::running() else {
        return;
    };
    for (stage, name) in RuntimeStage::GPU.into_iter().zip(PLOTS.iter()) {
        if let Some(elapsed) = frame.get(stage) {
            client.plot(*name, elapsed.as_secs_f64() * 1_000.0);
        }
    }
}

/// Marks the render thread's frame boundary once it has handed the frame to presentation;
/// render cleanup follows, and the present itself may still run on Bevy's present thread.
pub(super) fn install(app: &mut bevy::app::SubApp) {
    use bevy::prelude::*;
    use bevy::render::{Render, RenderSystems, view::window::present_frames};
    app.add_systems(
        Render,
        frame_mark
            .after(present_frames)
            .in_set(RenderSystems::Render),
    );
}

/// Uses Tracy's default frame series at the render thread's frame boundary.
fn frame_mark() {
    if let Some(client) = Client::running() {
        client.frame_mark();
    }
}
