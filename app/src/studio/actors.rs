//! Samples prepared actor draws without re-running animation, live selection or network input.

use std::sync::Arc;

use bevy::prelude::*;
use recording_scene::actors::{
    ActorCapture, ActorCoverage, ActorInstance, ActorSample, PreparedAppearance, PreparedGeometry,
};
use recording_studio::{MediaTime, capture::CaptureSession};
use render_scene::actor::{ActorRenderFrame, ActorRenderInstance};

use super::Active;

pub(super) struct NativeActors {
    pub capture: ActorCapture,
    geometry: Arc<PreparedGeometry>,
    appearance: Arc<PreparedAppearance>,
    geometry_key: (u64, u64, usize, usize, usize),
    appearance_key: (usize, usize),
    last_frame: u64,
}

/// Retains the current prepared frame as the take's zero-time baseline.
pub(super) fn start(world: &World, session: &CaptureSession) -> Result<NativeActors, String> {
    let frame = recording_frame(world)?;
    let mut capture = ActorCapture::start(session.directory(), session.info().clone())?;
    let geometry = capture.geometry(
        &frame.rig.geometry_vertices,
        Arc::clone(&frame.rig.geometry_spans),
    )?;
    let appearance = capture.appearance(Arc::clone(&frame.skins), Arc::clone(&frame.artwork))?;
    capture.sample(
        MediaTime::ZERO,
        snapshot(frame),
        Arc::clone(&geometry),
        Arc::clone(&appearance),
    )?;
    Ok(NativeActors {
        capture,
        geometry,
        appearance,
        geometry_key: geometry_key(frame),
        appearance_key: appearance_key(frame),
        last_frame: 0,
    })
}

/// Reuses immutable epoch leases at video cadence, admitting dynamic buffers through the bounded queue.
pub(super) fn sample(world: &World, active: &mut Active, at: MediaTime) -> Result<(), String> {
    let number = active
        .session
        .settings()
        .fps
        .frame_at(at)
        .map_err(|error| error.to_string())?;
    let Some(actors) = &mut active.actors else {
        return Ok(());
    };
    if actors.last_frame == number {
        return Ok(());
    }
    let frame = recording_frame(world)?;
    let geometry_key = geometry_key(frame);
    if actors.geometry_key != geometry_key {
        actors.geometry = actors.capture.geometry(
            &frame.rig.geometry_vertices,
            Arc::clone(&frame.rig.geometry_spans),
        )?;
        actors.geometry_key = geometry_key;
    }
    let appearance_key = appearance_key(frame);
    if actors.appearance_key != appearance_key {
        actors.appearance = actors
            .capture
            .appearance(Arc::clone(&frame.skins), Arc::clone(&frame.artwork))?;
        actors.appearance_key = appearance_key;
    }
    actors.capture.sample(
        at,
        snapshot(frame),
        Arc::clone(&actors.geometry),
        Arc::clone(&actors.appearance),
    )?;
    actors.last_frame = number;
    Ok(())
}

/// Uses the recording-only native publication, which includes received actors and the local world body.
fn snapshot(frame: &ActorRenderFrame) -> ActorSample {
    let ActorRenderFrame {
        instances,
        skins: _,
        instance_revision,
        skin_revision,
        rig,
        artwork: _,
        instance_pages,
    } = frame;
    let instances = instances
        .iter()
        .map(|instance| {
            let &ActorRenderInstance {
                runtime_id,
                position,
                pitch_radians,
                yaw_radians,
                head_yaw_radians,
                skin_layer,
            } = instance;
            ActorInstance {
                runtime_id,
                position,
                pitch_radians,
                yaw_radians,
                head_yaw_radians,
                skin_layer,
            }
        })
        .collect::<Vec<_>>()
        .into();
    ActorSample {
        coverage: ActorCoverage::ReceivedWorld,
        instances,
        instance_revision: *instance_revision,
        skin_revision: *skin_revision,
        rig: rig.clone(),
        instance_pages: Arc::clone(instance_pages),
    }
}

/// Missing complete publication interrupts capture rather than relabeling a culled live frame.
fn recording_frame(world: &World) -> Result<&ActorRenderFrame, String> {
    world
        .get_resource::<client_presentation::actor_publication::recording::ReplayActors>()
        .filter(|actors| actors.enabled)
        .and_then(|actors| actors.frame.as_ref())
        .ok_or_else(|| "The received-world actor recording frame is unavailable.".into())
}

/// Leases retain the old allocations, so pointer identity cannot be reused before epoch replacement.
fn geometry_key(frame: &ActorRenderFrame) -> (u64, u64, usize, usize, usize) {
    (
        frame.rig.geometry_vertices.epoch,
        frame.rig.geometry_revision,
        Arc::as_ptr(&frame.rig.geometry_vertices.segments) as *const () as usize,
        Arc::as_ptr(&frame.rig.geometry_vertices.offsets) as *const () as usize,
        Arc::as_ptr(&frame.rig.geometry_spans) as *const () as usize,
    )
}

/// Native publications replace immutable raster storage when its contents change.
fn appearance_key(frame: &ActorRenderFrame) -> (usize, usize) {
    (
        Arc::as_ptr(&frame.skins) as usize,
        Arc::as_ptr(&frame.artwork) as usize,
    )
}
