//! Frozen prepared actor poses and appearances. Coverage is explicit before independent cameras use them.

use std::{io::Read, path::Path, sync::Arc};

use recording_studio::{
    MediaTime,
    archive::{ArchiveError, RecordKind, RecordingCursor, RecordingReader, ReplayRecord},
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

#[derive(Clone)]
pub struct FrozenActors {
    pub sample: ActorSample,
    pub skins: Arc<ActorSkinResidency>,
    pub artwork: Arc<ActorArtworkPages>,
    pub geometry_identity: AssetId,
    pub appearance_identity: AssetId,
}

pub struct ActorRecording {
    cursor: RecordingCursor,
    store: AssetReader,
    geometry: Option<(AssetId, Arc<geometry::LoadedGeometry>)>,
    appearance: Option<(AssetId, Arc<appearance::LoadedAppearance>)>,
    pending_bytes: Vec<u8>,
    current: Option<FrozenActors>,
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
            cursor: reader.into_cursor(),
            store: AssetReader::new(&root),
            geometry: None,
            appearance: None,
            pending_bytes: Vec::new(),
            current: None,
        })
    }

    /// Admits a complete independent frame. Coverage remains attached so a caller cannot imply unseen actors.
    pub fn restore(
        &mut self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
    ) -> Result<FrozenActors, ArchiveError> {
        let mut found = false;
        let bytes = &mut self.pending_bytes;
        let result = self
            .cursor
            .seek(at, &cancelled, |record| {
                latest_pose(record, &mut found, bytes)
            })
            .and_then(|()| {
                if found {
                    self.load_pending(&cancelled)
                } else {
                    Err(ArchiveError::Invalid(
                        "actor recording has no prepared frame",
                    ))
                }
            });
        match result {
            Ok(frame) => {
                self.current = Some(frame.clone());
                Ok(frame)
            }
            Err(error) => {
                self.cursor.invalidate();
                Err(error)
            }
        }
    }

    /// Advances only new pose records, decoding the last crossed pose once even after a playback jump.
    pub fn advance_to(
        &mut self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
    ) -> Result<&FrozenActors, ArchiveError> {
        let mut found = false;
        let bytes = &mut self.pending_bytes;
        let result = self
            .cursor
            .advance_to(at, &cancelled, |record| {
                latest_pose(record, &mut found, bytes)
            })
            .and_then(|()| {
                if found {
                    self.load_pending(&cancelled).map(Some)
                } else {
                    Ok(None)
                }
            });
        match result {
            Ok(Some(frame)) => self.current = Some(frame),
            Ok(None) => {}
            Err(error) => {
                self.cursor.invalidate();
                return Err(error);
            }
        }
        self.current.as_ref().ok_or(ArchiveError::Invalid(
            "actor playback needs a successful seek",
        ))
    }

    /// Scene admission and journal progress share one validity boundary.
    pub fn invalidate(&mut self) {
        self.cursor.invalidate();
    }

    /// Track duration comes from its verified completion or recoverable committed prefix.
    pub fn duration(&self) -> MediaTime {
        self.cursor.reader().duration()
    }

    /// A scene coordinator checks that every track belongs to the same take before playback.
    pub fn info(&self) -> &recording_studio::archive::RecordingInfo {
        self.cursor.reader().info()
    }

    /// Only the selected pose is decoded; immutable geometry and appearance epochs reuse validated caches.
    fn load_pending(
        &mut self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<FrozenActors, ArchiveError> {
        let pending = pose::read(&mut &self.pending_bytes[..])?;
        let geometry = match &self.geometry {
            Some((id, geometry)) if *id == pending.geometry => Arc::clone(geometry),
            _ => {
                let geometry = Arc::new(geometry::load(&self.store, pending.geometry, cancelled)?);
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
                    cancelled,
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

/// Copies bounded compressed bytes rather than repeatedly decoding every skipped frame during a seek.
fn latest_pose(
    record: ReplayRecord<'_>,
    found: &mut bool,
    bytes: &mut Vec<u8>,
) -> Result<(), ArchiveError> {
    match record.kind {
        RecordKind::CheckpointStart => {
            *found = false;
            bytes.clear();
        }
        RecordKind::CheckpointEnd => {}
        RecordKind::CheckpointPart | RecordKind::Scene => {
            bytes.clear();
            bytes.extend_from_slice(record.payload);
            *found = true;
        }
        _ => return Err(ArchiveError::Invalid("unexpected prepared-actor record")),
    }
    Ok(())
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
