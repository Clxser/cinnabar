//! Frozen prepared actor poses and appearances. Coverage is explicit before independent cameras use them.

use std::{io::Read, path::Path, sync::Arc};

use recording_studio::{
    MediaTime,
    archive::{ArchiveError, RecordKind, RecordingReader},
    object_store::{AssetId, AssetReader},
};
use render_model::{
    ActorArtworkPageId, ActorArtworkPages, ActorRigRenderFrame, ActorSkinResidency,
};
use serde::{Deserialize, Serialize};

mod appearance;
mod capture;
mod geometry;
mod metadata;
mod pose;
pub use capture::{ActorCapture, PreparedAppearance, PreparedGeometry};

pub const ACTOR_TRACK_SCHEMA: &str = "cinnabar.prepared-actors/1";
pub const ACTOR_DIRECTORY: &str = "actors.cinreplay";

/// The original camera's draw list cannot supply actors that were culled before publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorCoverage {
    OriginalView,
    ReceivedWorld,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorInstance {
    pub runtime_id: u64,
    pub position: [f32; 3],
    pub pitch_radians: f32,
    pub yaw_radians: f32,
    pub head_yaw_radians: f32,
    pub skin_layer: u32,
}

#[derive(Clone)]
pub struct ActorSample {
    pub coverage: ActorCoverage,
    pub instances: Arc<[ActorInstance]>,
    pub instance_revision: u64,
    pub skin_revision: u64,
    pub rig: ActorRigRenderFrame,
    pub instance_pages: Arc<[ActorArtworkPageId]>,
}

impl ActorSample {
    /// Only dynamic buffers are charged per queued sample; geometry and rasters have separate leases.
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + std::mem::size_of_val(&*self.instances)
            + std::mem::size_of_val(&*self.rig.instances)
            + std::mem::size_of_val(&*self.rig.previous_bones)
            + std::mem::size_of_val(&*self.rig.current_bones)
            + std::mem::size_of_val(&*self.rig.manifest)
            + std::mem::size_of_val(&*self.instance_pages)
    }
}

pub struct FrozenActors {
    pub sample: ActorSample,
    pub skins: Arc<ActorSkinResidency>,
    pub artwork: Arc<ActorArtworkPages>,
    pub geometry_identity: AssetId,
    pub appearance_identity: AssetId,
}

pub struct ActorRecording {
    reader: RecordingReader,
    store: AssetReader,
    geometry: Option<(AssetId, Arc<geometry::LoadedGeometry>)>,
    appearance: Option<(AssetId, Arc<appearance::LoadedAppearance>)>,
}

impl ActorRecording {
    /// Opens the explicitly versioned prepared-actor track without borrowing a live actor authority.
    pub fn open(parent: &Path) -> Result<Self, ArchiveError> {
        let root = parent.join(ACTOR_DIRECTORY);
        let reader = RecordingReader::open(&root)?;
        if reader.info().scene_schema != ACTOR_TRACK_SCHEMA {
            return Err(ArchiveError::Invalid("unsupported actor recording schema"));
        }
        Ok(Self {
            reader,
            store: AssetReader::new(&root),
            geometry: None,
            appearance: None,
        })
    }

    /// Admits a complete independent frame. Coverage remains attached so a caller cannot imply unseen actors.
    pub fn restore(
        &mut self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
    ) -> Result<FrozenActors, ArchiveError> {
        let mut latest = None;
        self.reader.visit_to(at, &cancelled, |record| {
            match record.kind {
                RecordKind::CheckpointStart => latest = None,
                RecordKind::CheckpointEnd => {}
                RecordKind::CheckpointPart | RecordKind::Scene => {
                    latest = Some(pose::read(&mut &record.payload[..])?)
                }
                _ => return Err(ArchiveError::Invalid("unexpected prepared-actor record")),
            }
            Ok(())
        })?;
        let pending = latest.ok_or(ArchiveError::Invalid(
            "actor recording has no prepared frame",
        ))?;
        let geometry = match &self.geometry {
            Some((id, geometry)) if *id == pending.geometry => Arc::clone(geometry),
            _ => {
                let geometry = Arc::new(geometry::load(&self.store, pending.geometry, &cancelled)?);
                self.geometry = Some((pending.geometry, Arc::clone(&geometry)));
                geometry
            }
        };
        let appearance = match &self.appearance {
            Some((id, appearance)) if *id == pending.appearance => Arc::clone(appearance),
            _ => {
                let appearance = Arc::new(appearance::load(
                    &self.store,
                    pending.appearance,
                    &cancelled,
                )?);
                self.appearance = Some((pending.appearance, Arc::clone(&appearance)));
                appearance
            }
        };
        let frame = pending.admit(&geometry, &appearance)?;
        if cancelled() {
            return Err(ArchiveError::Cancelled);
        }
        Ok(frame)
    }
}

/// All record IO uses bounded readers; decoding failures remain local recording errors.
fn io_error(_: std::io::Error) -> ArchiveError {
    ArchiveError::Invalid("truncated or unwritable prepared-actor data")
}

/// Fixed magic headers separate independently versioned static asset and pose layouts.
fn magic(input: &mut dyn Read, expected: [u8; 8]) -> Result<(), ArchiveError> {
    let mut actual = [0; 8];
    input.read_exact(&mut actual).map_err(io_error)?;
    if actual != expected {
        return Err(ArchiveError::Invalid(
            "unsupported prepared-actor asset layout",
        ));
    }
    Ok(())
}
