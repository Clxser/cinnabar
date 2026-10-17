//! Captures already prepared native environment inputs without changing live lighting or shader state.

use std::sync::Arc;

use bevy::prelude::*;
use recording_scene::environment::{
    EnvironmentCapture, EnvironmentState, PreparedEnvironmentAssets, RecordedEnhanced,
    RecordedEnvironmentView,
};
use recording_studio::{MediaTime, capture::CaptureSession};
use render_model::environment_snapshot::LightmapSnapshot;
use render_scene::{
    ChunkAnimationClock,
    atmosphere::{AtmosphereFrame, AtmosphereTextureAssets},
    lighting::{WorldFullbright, WorldLighting},
};

use super::Active;

pub(super) struct NativeEnvironment {
    pub capture: EnvironmentCapture,
    assets: Arc<PreparedEnvironmentAssets>,
    texture_identity: (usize, [u8; 32]),
    last_frame: u64,
}

/// Starts an independent track with the exact state visible when this take's capture clock begins.
pub(super) fn start(world: &World, session: &CaptureSession) -> Result<NativeEnvironment, String> {
    let mut capture = EnvironmentCapture::start(session.directory(), session.info().clone())?;
    let textures = world
        .get_resource::<AtmosphereTextureAssets>()
        .ok_or("Atmosphere textures are unavailable for recording.")?;
    let runtime = textures
        .runtime()
        .ok_or("Atmosphere recording requires the fetched sky carrier.")?;
    let texture_identity = (Arc::as_ptr(runtime) as usize, textures.identity());
    let assets = capture.assets(Arc::clone(runtime), textures.shared_skyboxes())?;
    capture.sample(MediaTime::ZERO, state(world)?, Arc::clone(&assets))?;
    Ok(NativeEnvironment {
        capture,
        assets,
        texture_identity,
        last_frame: 0,
    })
}

/// Samples at the take's video cadence, retaining immutable texture epochs across many small records.
pub(super) fn sample(world: &World, active: &mut Active, at: MediaTime) -> Result<(), String> {
    let frame = active
        .session
        .settings()
        .fps
        .frame_at(at)
        .map_err(|error| error.to_string())?;
    let Some(environment) = &mut active.environment else {
        return Ok(());
    };
    if frame == environment.last_frame {
        return Ok(());
    }
    let textures = world
        .get_resource::<AtmosphereTextureAssets>()
        .ok_or("Recorded atmosphere textures were removed.")?;
    let runtime = textures
        .runtime()
        .ok_or("The recorded atmosphere carrier was removed.")?;
    let texture_identity = (Arc::as_ptr(runtime) as usize, textures.identity());
    if environment.texture_identity != texture_identity {
        environment.assets = environment
            .capture
            .assets(Arc::clone(runtime), textures.shared_skyboxes())?;
        environment.texture_identity = texture_identity;
    }
    environment
        .capture
        .sample(at, state(world)?, Arc::clone(&environment.assets))?;
    environment.last_frame = frame;
    Ok(())
}

/// Copies only owner-typed prepared values, avoiding clock reads and environment inference here.
fn state(world: &World) -> Result<EnvironmentState, String> {
    let atmosphere = world
        .get_resource::<AtmosphereFrame>()
        .ok_or("Prepared atmosphere is unavailable for recording.")?
        .snapshot();
    let lighting = world
        .get_resource::<WorldLighting>()
        .ok_or("Prepared lighting is unavailable for recording.")?;
    let animation = world
        .get_resource::<ChunkAnimationClock>()
        .ok_or("Texture animation clock is unavailable for recording.")?;
    let &ChunkAnimationClock {
        tick,
        partial_tick,
        _padding_0: _,
        _padding_1: _,
    } = animation;
    let fullbright = world
        .get_resource::<WorldFullbright>()
        .copied()
        .unwrap_or_default()
        .0;
    Ok(EnvironmentState {
        atmosphere,
        lightmap: LightmapSnapshot::capture(lighting.0),
        fullbright,
        animation_tick: tick,
        animation_partial: partial_tick,
        view: Some(recorded_view(world)?),
    })
}

/// Captures the actual gameplay camera component and the authoritative world dimension.
fn recorded_view(world: &World) -> Result<RecordedEnvironmentView, String> {
    let dimension = world
        .get_resource::<crate::runtime::world::ClientWorld>()
        .and_then(|world| world.stream.as_ref())
        .map(|stream| stream.current_dimension())
        .ok_or("The recorded dimension is unavailable.")?;
    let camera = world
        .iter_entities()
        .find(|entity| entity.contains::<client_presentation::camera::FlyCamera>())
        .ok_or("The recorded gameplay camera is unavailable.")?;
    let enhanced = camera
        .get::<render_scene::EnhancedRendering>()
        .map(|options| {
            let &render_scene::EnhancedRendering {
                shadows,
                shadow_resolution,
                shadow_cascades,
                shadow_distance,
                bloom,
                light_shafts,
                waving,
                water_reflections,
            } = options;
            RecordedEnhanced {
                shadows,
                shadow_resolution,
                shadow_cascades,
                shadow_distance,
                bloom,
                light_shafts,
                waving,
                water_reflections,
            }
        });
    let local_body_visible = world
        .get_resource::<client_presentation::local_player::LocalAvatarVisibilityCarrier>()
        .and_then(|carrier| carrier.snapshot())
        .is_some_and(|snapshot| snapshot.visible());
    Ok(RecordedEnvironmentView {
        dimension,
        enhanced,
        local_body_visible,
    })
}
