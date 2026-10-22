//! Independent native scene and animation caches for bounded received-world recording.

use super::*;
use recording_scene::visuals::BlockCoverage;
mod items;

/// Column metadata remains bounded even in a server with a very large retained world.
pub(super) const MAX_CACHED_COLUMNS: usize = recording_scene::scene::MAX_SCENE_CHUNKS;

#[derive(Resource)]
pub(crate) struct ReplayBlockEntities {
    enabled: bool,
    requested: bool,
    runtime: BlockEntityRuntime,
    scene: BlockEntityScene,
    previous_seconds: Option<f64>,
    placements: Vec<StaticItemPlacement>,
    pub(crate) frame: Option<render_model::BlockEntityFrame>,
    pub(crate) coverage: Option<BlockCoverage>,
}

impl Default for ReplayBlockEntities {
    fn default() -> Self {
        Self {
            enabled: false,
            requested: false,
            runtime: BlockEntityRuntime::new(),
            scene: BlockEntityScene::default(),
            previous_seconds: None,
            placements: Vec::new(),
            frame: None,
            coverage: None,
        }
    }
}

impl ReplayBlockEntities {
    /// Capture admission owns the lifetime of all extra animation, canvas and column state.
    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        if self.enabled && !enabled {
            *self = Self::default();
        }
        self.enabled = enabled;
    }

    /// Requests one received-world sample before this frame's native preparation.
    pub(crate) fn request_frame(&mut self) {
        self.requested = self.enabled;
    }
}

/// Reuses the native builder with separate clocks/canvases and no source-camera selection.
#[allow(clippy::too_many_arguments)]
pub(super) fn update(
    client_world: Res<ClientWorld>,
    actor_partial_tick: Res<ActorFramePartialTick>,
    collisions: Res<PhysicsCollisionRegistries>,
    view: Res<LocalViewPose>,
    ui: Res<UiRuntimeResource>,
    atmosphere: Res<AtmosphereFrame>,
    time: Res<Time<Real>>,
    font: Res<BlockEntityFont>,
    source_runtime: Res<BlockEntityRuntime>,
    source_scene: Res<render_scene::BlockEntitySceneResource>,
    mut replay: ResMut<ReplayBlockEntities>,
    mut items: items::Items,
) {
    if !replay.enabled || !std::mem::take(&mut replay.requested) {
        return;
    }
    let now = time.elapsed_secs_f64();
    let replay = &mut *replay;
    let session = client_world.stream.as_ref().map(|stream| {
        (
            stream.authority().actor_session_id(),
            stream.current_dimension(),
        )
    });
    let first = replay.previous_seconds.is_none() || replay.runtime.session != session;
    if first {
        replay.runtime.bind_session(session);
        // Already visible animations begin at their real source pose, with no second live tick.
        replay.runtime.lids = source_runtime.lids.clone();
        replay.runtime.books = source_runtime.books.clone();
        replay.runtime.bell_rings = source_runtime.bell_rings.clone();
    }
    replay.scene.sync_prepared_sources(&source_scene);
    let delta = if first {
        0.0
    } else {
        replay
            .previous_seconds
            .map_or(0.0, |previous| (now - previous).max(0.0) as f32)
    };
    let mut frame = render_model::BlockEntityFrame::default();
    let mut placements = std::mem::take(&mut replay.placements);
    let coverage = frame::prepare(
        client_world.stream.as_ref(),
        actor_partial_tick.0,
        None,
        &collisions,
        view.eye_translation(),
        &ui,
        &atmosphere,
        now,
        delta,
        &font.0,
        &mut replay.runtime,
        &mut replay.scene,
        &mut frame,
        &mut placements,
        None,
        None,
        true,
    );
    replay.previous_seconds = Some(now);
    items.prepare(
        client_world.stream.as_ref(),
        &collisions,
        actor_partial_tick.0,
        now,
        &placements,
    );
    placements.clear();
    replay.placements = placements;
    replay.frame = Some(frame);
    replay.coverage = Some(coverage);
}
