use bevy::{
    prelude::{Deref, DerefMut, Resource},
    render::extract_resource::ExtractResource,
};
use render_model::{UiRenderScene, UiRenderStats};
/// Main-world holder of the published [`UiRenderScene`], cloned into the render world.
#[derive(Resource, ExtractResource, Clone, Debug, Default, Deref, DerefMut)]
#[extract_app(bevy::render::RenderApp)]
pub struct UiRenderSceneResource(pub UiRenderScene);

/// The [`UiRenderStats`] handle both worlds share.
#[derive(Resource, Clone, Debug, Default, Deref, DerefMut)]
pub struct UiRenderStatsResource(pub UiRenderStats);

/// Capture eligibility travels beside the same extracted UI publication as its rendered scene.
#[derive(Resource, ExtractResource, Clone, Copy, Debug, Default)]
#[extract_app(bevy::render::RenderApp)]
pub struct UiHudCaptureScope {
    pub enabled: bool,
    pub allowed: bool,
}

/// Replay supplies the prepared shader phase; ordinary UI keeps its renderer-owned clock.
#[derive(Resource, ExtractResource, Clone, Copy, Debug, Default)]
#[extract_app(bevy::render::RenderApp)]
pub struct RecordedUiTime(pub Option<f32>);

/// One immutable prepared frame replaces the prior one; producer and consumer never wait on a lock.
#[derive(Resource, Clone, Default)]
pub struct UiPreparedHud(
    std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<render_model::PreparedUiFrame>>>>,
);

impl UiPreparedHud {
    /// Contention skips this publication rather than blocking render or gameplay.
    pub fn publish(&self, frame: Option<render_model::PreparedUiFrame>) {
        if let Ok(mut latest) = self.0.try_lock() {
            *latest = frame.map(std::sync::Arc::new);
        }
    }

    /// Capture borrows only the most recently admitted, explicitly eligible rendered HUD.
    pub fn latest(&self) -> Option<std::sync::Arc<render_model::PreparedUiFrame>> {
        self.0.try_lock().ok()?.clone()
    }
}
