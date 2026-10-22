//! Samples prepared actor draws without re-running animation, live selection or network input.

use std::sync::Arc;

use bevy::prelude::*;
use recording_scene::actors::{
    ActorCapture, ActorCoverage, ActorSample, PreparedAppearance, PreparedGeometry,
};
use recording_studio::{MediaTime, capture::CaptureSession};
use render_scene::actor::ActorRenderFrame;

use super::Active;

mod instances;

pub(super) struct NativeActors {
    pub capture: ActorCapture,
    geometry: Arc<PreparedGeometry>,
    appearance: Arc<PreparedAppearance>,
    geometry_key: (u64, u64, usize, usize, usize),
    appearance_key: AppearanceKey,
    instances: instances::Projection,
    last_frame: u64,
}

#[derive(PartialEq, Eq)]
struct AppearanceKey {
    skins: usize,
    identity: [u8; 32],
    entity_identity: [u8; 32],
    pages: usize,
    actor_glint: Option<(u16, u16, usize)>,
    item_glint: Option<(u16, u16, usize)>,
}

/// Retains the current prepared frame as the take's zero-time baseline.
pub(super) fn start(world: &World, session: &CaptureSession) -> Result<NativeActors, String> {
    let frame = recording_frame(world)?;
    let mut capture = ActorCapture::start_with_disk(
        session.directory(),
        session.info().clone(),
        session.disk_budget().clone(),
    )?;
    let geometry = capture.geometry(
        &frame.rig.geometry_vertices,
        Arc::clone(&frame.rig.geometry_spans),
    )?;
    let appearance = capture.appearance(Arc::clone(&frame.skins), Arc::clone(&frame.artwork))?;
    let mut instances = instances::Projection::default();
    capture.sample(
        MediaTime::ZERO,
        snapshot(frame, &mut instances),
        Arc::clone(&geometry),
        Arc::clone(&appearance),
    )?;
    Ok(NativeActors {
        capture,
        geometry,
        appearance,
        geometry_key: geometry_key(frame),
        appearance_key: appearance_key(frame),
        instances,
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
        snapshot(frame, &mut actors.instances),
        Arc::clone(&actors.geometry),
        Arc::clone(&actors.appearance),
    )?;
    actors.last_frame = number;
    Ok(())
}

/// Uses the recording-only native publication, which includes received actors and the local world body.
fn snapshot(frame: &ActorRenderFrame, cache: &mut instances::Projection) -> ActorSample {
    let ActorRenderFrame {
        mask_only_runtime_ids: _,
        instances,
        skins: _,
        instance_revision,
        skin_revision,
        rig,
        artwork: _,
        instance_pages,
    } = frame;
    ActorSample {
        coverage: ActorCoverage::ReceivedWorld,
        instances: cache.project(instances),
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

/// Recreated artwork wrappers reuse a lease when all serialized raster storage remains unchanged.
fn appearance_key(frame: &ActorRenderFrame) -> AppearanceKey {
    let artwork = &frame.artwork;
    let raster_key = |raster: &render_model::EquipmentRaster| {
        (raster.width, raster.height, raster.rgba8.as_ptr() as usize)
    };
    AppearanceKey {
        skins: Arc::as_ptr(&frame.skins) as usize,
        identity: artwork.identity,
        entity_identity: artwork.entity_identity,
        pages: Arc::as_ptr(&artwork.pages) as *const () as usize,
        actor_glint: artwork.actor_glint.as_ref().map(raster_key),
        item_glint: artwork.item_glint.as_ref().map(raster_key),
    }
}
