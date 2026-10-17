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
pub(crate) mod geometry;
mod interpolate;
mod metadata;
pub(crate) mod pose;
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
    current: Option<(MediaTime, FrozenActors)>,
    next: Option<(MediaTime, FrozenActors)>,
    sampled: Option<FrozenActors>,
    position: Option<MediaTime>,
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
            next: None,
            sampled: None,
            position: None,
        })
    }

    /// Admits a complete independent frame. Coverage remains attached so a caller cannot imply unseen actors.
    pub fn restore(
        &mut self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
    ) -> Result<FrozenActors, ArchiveError> {
        let result = (|| {
            self.current = Some(self.read_pose(at, true, &cancelled)?.ok_or(
                ArchiveError::Invalid("actor recording has no prepared frame"),
            )?);
            self.read_next(&cancelled)?;
            self.publish(at, &cancelled)
        })();
        if let Err(error) = result {
            self.invalidate();
            return Err(error);
        }
        self.sampled.clone().ok_or(ArchiveError::Invalid(
            "actor playback needs a successful seek",
        ))
    }

    /// Advances adjacent source poses once and samples the requested time between compatible draws.
    pub fn advance_to(
        &mut self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
    ) -> Result<&FrozenActors, ArchiveError> {
        if cancelled() {
            self.invalidate();
            return Err(ArchiveError::Cancelled);
        }
        if self.position.is_none_or(|previous| at < previous) || at > self.duration() {
            return Err(ArchiveError::Invalid(
                "actor playback needs a seek before advancing",
            ));
        }
        if self.position == Some(at) {
            return self.sampled.as_ref().ok_or(ArchiveError::Invalid(
                "actor playback needs a successful seek",
            ));
        }
        let result = (|| {
            if self.next.as_ref().is_some_and(|(time, _)| *time <= at) {
                self.current = self.next.take();
                if let Some(frame) = self.read_pose(at, false, &cancelled)? {
                    self.current = Some(frame);
                }
                self.read_next(&cancelled)?;
            }
            self.publish(at, &cancelled)
        })();
        if let Err(error) = result {
            self.invalidate();
            return Err(error);
        }
        self.sampled.as_ref().ok_or(ArchiveError::Invalid(
            "actor playback needs a successful seek",
        ))
    }

    /// Scene admission and journal progress share one validity boundary.
    pub fn invalidate(&mut self) {
        self.cursor.invalidate();
        self.current = None;
        self.next = None;
        self.sampled = None;
        self.position = None;
    }

    /// Track duration comes from its verified completion or recoverable committed prefix.
    pub fn duration(&self) -> MediaTime {
        self.cursor.reader().duration()
    }

    /// A scene coordinator checks that every track belongs to the same take before playback.
    pub fn info(&self) -> &recording_studio::archive::RecordingInfo {
        self.cursor.reader().info()
    }

    /// One cursor retains only the admitted current pose, one future pose and a reusable payload.
    fn read_pose(
        &mut self,
        at: MediaTime,
        seek: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<(MediaTime, FrozenActors)>, ArchiveError> {
        let mut found = None;
        let bytes = &mut self.pending_bytes;
        let mut visit = |record: ReplayRecord<'_>| latest_pose(record, &mut found, bytes);
        if seek {
            self.cursor.seek(at, cancelled, &mut visit)?;
        } else {
            self.cursor.advance_to(at, cancelled, &mut visit)?;
        }
        found
            .map(|time| self.load_pending(cancelled).map(|frame| (time, frame)))
            .transpose()
    }

    /// Multipart checkpoints and any non-pose boundaries are consumed at their complete timestamp.
    fn read_next(&mut self, cancelled: &impl Fn() -> bool) -> Result<(), ArchiveError> {
        self.next = None;
        while let Some(at) = self.cursor.next_time(cancelled)? {
            if let Some(frame) = self.read_pose(at, false, cancelled)? {
                self.next = Some(frame);
                break;
            }
        }
        Ok(())
    }

    /// Source-time revisions invalidate GPU uploads identically for seeks, playback and offline exports.
    fn publish(
        &mut self,
        at: MediaTime,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), ArchiveError> {
        let (time, current) = self.current.as_ref().ok_or(ArchiveError::Invalid(
            "actor playback needs a successful seek",
        ))?;
        let mut frame = if let Some((next_at, next)) = &self.next {
            let amount = at.saturating_sub(*time).0 as f64 / next_at.saturating_sub(*time).0 as f64;
            interpolate::sample(current, next, amount as f32, cancelled)?
        } else {
            current.clone()
        };
        if cancelled() {
            return Err(ArchiveError::Cancelled);
        }
        frame.sample.instance_revision = at.0 + 1;
        frame.sample.rig.frame_generation = at.0 + 1;
        self.sampled = Some(frame);
        self.position = Some(at);
        Ok(())
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
    found: &mut Option<MediaTime>,
    bytes: &mut Vec<u8>,
) -> Result<(), ArchiveError> {
    match record.kind {
        RecordKind::CheckpointStart => {
            *found = None;
            bytes.clear();
        }
        RecordKind::CheckpointEnd => {}
        RecordKind::CheckpointPart | RecordKind::Scene => {
            bytes.clear();
            bytes.extend_from_slice(record.payload);
            *found = Some(record.at);
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
