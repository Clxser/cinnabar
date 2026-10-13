//! Installs offscreen device measurements beside the production terrain renderer.

mod runner;
mod scene;

use bevy::{
    prelude::*,
    render::{Render, RenderApp, extract_resource::ExtractResourcePlugin},
};
use render_scene::graphics_benchmark::GraphicsBenchmark;

/// Shares one mailbox and registers benchmark work before the renderer's single device poll.
pub(super) fn install(app: &mut App) {
    app.init_resource::<GraphicsBenchmark>()
        .add_plugins(ExtractResourcePlugin::<GraphicsBenchmark>::default());
    let render = app.sub_app_mut(RenderApp);
    crate::device_poll::install(render);
    render.init_resource::<runner::State>().add_systems(
        Render,
        runner::drive.in_set(crate::device_poll::FrameSubmissions),
    );
}

#[cfg(test)]
mod tests;
