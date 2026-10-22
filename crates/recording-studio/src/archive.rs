//! Indexed local replay journals. Frame adapters supply immutable payloads and caller-owned time;
//! compression, checksums, file publication and recovery belong to the recording worker.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use crate::{MediaTime, time::MAX_MEDIA_MICROS};

#[cfg(test)]
mod compression_tests;
mod format;
mod reader;
mod task;
mod writer;

pub use reader::{RecordingCursor, RecordingReader, Recovery, ReplayRecord};
pub use task::{Recording, RecordingProgress, RecordingState, RecordingTask};

pub const RECORDING_EXTENSION: &str = "cinreplay";
pub const RECORDING_VERSION: u32 = 1;
pub const MAX_RECORD_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SEGMENTS: usize = 65_536;
pub const MAX_CHECKPOINT_BYTES: u64 = 512 * 1024 * 1024;
/// Native capture owners share this seek interval while retaining independently versioned payloads.
pub const SCENE_CHECKPOINT_INTERVAL: MediaTime = MediaTime(5_000_000);
const MAX_METADATA_BYTES: usize = 64 * 1024;
const METADATA_FILE: &str = "recording.json";
const INDEX_FILE: &str = "index.bin";
const COMPLETE_FILE: &str = "complete.json";

/// The schema identifies the owning scene adapter; it is checked before any payload is interpreted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingInfo {
    pub version: u32,
    pub name: String,
    pub game_version: String,
    pub protocol_version: u32,
    pub scene_schema: String,
    pub created_unix_ms: u64,
    pub viewport: [u32; 2],
    pub local_actor: u64,
}

impl RecordingInfo {
    /// Reads bounded library metadata without decompressing scene data or loading the replay index.
    pub fn read(root: &Path) -> Result<Self, ArchiveError> {
        let bytes = format::read_bounded(&root.join(METADATA_FILE), MAX_METADATA_BYTES)?;
        let info: Self = serde_json::from_slice(&bytes)?;
        info.validate()?;
        Ok(info)
    }

    /// Rejects unsupported containers and unbounded metadata before starting IO or loading a scene.
    pub fn validate(&self) -> Result<(), ArchiveError> {
        if self.version != RECORDING_VERSION {
            return Err(ArchiveError::Version(self.version));
        }
        if self.name.is_empty()
            || self.name.len() > 256
            || self.game_version.is_empty()
            || self.game_version.len() > 64
            || self.scene_schema.is_empty()
            || self.scene_schema.len() > 128
            || self.viewport.iter().any(|&side| side == 0 || side > 32_768)
        {
            return Err(ArchiveError::Invalid("invalid recording metadata"));
        }
        Ok(())
    }
}

// The on-disk discriminant and decoder come from the same schema declaration.
macro_rules! record_kinds {
    ($($kind:ident = $number:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr(u8)]
        pub enum RecordKind { $($kind = $number),+ }

        impl TryFrom<u8> for RecordKind {
            type Error = ArchiveError;
            fn try_from(value: u8) -> Result<Self, Self::Error> {
                match value {
                    $($number => Ok(Self::$kind),)+
                    _ => Err(ArchiveError::Invalid("unknown replay record kind")),
                }
            }
        }
    };
}

record_kinds! {
    CheckpointStart = 1,
    CheckpointPart = 2,
    CheckpointEnd = 3,
    Scene = 4,
    Camera = 5,
    Gameplay = 6,
    Audio = 7,
    Asset = 8,
}

/// Checkpoints are multipart, all at one timestamp, so a large world never needs one giant buffer.
/// Between checkpoints, Scene records carry the owning adapter's ordered incremental changes.
#[derive(Clone, Debug)]
pub struct RecordInput {
    pub at: MediaTime,
    pub kind: RecordKind,
    pub payload: Arc<[u8]>,
}

impl RecordInput {
    /// Bounds an individual transfer before reserving queue memory.
    pub fn validate(&self) -> Result<(), ArchiveError> {
        if self.at.0 > MAX_MEDIA_MICROS || self.payload.len() > MAX_RECORD_BYTES {
            return Err(ArchiveError::Invalid("replay record exceeds its bounds"));
        }
        if matches!(
            self.kind,
            RecordKind::CheckpointStart | RecordKind::CheckpointEnd
        ) && !self.payload.is_empty()
        {
            return Err(ArchiveError::Invalid(
                "checkpoint boundaries have no payload",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RecordingConfig {
    pub queued_records: usize,
    pub queued_bytes: usize,
    pub segment_bytes: u64,
    pub segment_duration: MediaTime,
    pub disk_quota: u64,
    /// Zlib level for journal segments. Precompressed scene payloads use stored zlib blocks.
    pub compression_level: u32,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            queued_records: 256,
            queued_bytes: 64 * 1024 * 1024,
            segment_bytes: 4 * 1024 * 1024,
            segment_duration: MediaTime(1_000_000),
            disk_quota: 25 * 1024 * 1024 * 1024,
            compression_level: 1,
        }
    }
}

impl RecordingConfig {
    /// Queues and compression buffers stay bounded even when the destination disk stalls.
    pub fn validate(self) -> Result<(), ArchiveError> {
        if !(1..=4096).contains(&self.queued_records)
            || !(MAX_RECORD_BYTES..=1024 * 1024 * 1024).contains(&self.queued_bytes)
            || !(64 * 1024..=MAX_CHECKPOINT_BYTES).contains(&self.segment_bytes)
            || !(100_000..=10_000_000).contains(&self.segment_duration.0)
            || self.disk_quota < 1024 * 1024
            || self.compression_level > 9
        {
            return Err(ArchiveError::Invalid("invalid recording storage limits"));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("unsupported replay format version {0}")]
    Version(u32),
    #[error("recording stopped because its storage queue is full; committed data is recoverable")]
    Backpressure,
    #[error("recording is no longer accepting data")]
    Closed,
    #[error("replay seek was cancelled")]
    Cancelled,
    #[error("recording storage quota was reached")]
    Quota,
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("replay metadata cannot be decoded: {0}")]
    Json(#[from] serde_json::Error),
    #[error("replay segment {0} failed its integrity check")]
    Integrity(u32),
}

/// Keeps the affected file in every storage error shown by the library or recorder.
fn io(path: impl Into<PathBuf>, source: std::io::Error) -> ArchiveError {
    ArchiveError::Io {
        path: path.into(),
        source,
    }
}

#[derive(Clone, Copy, Debug)]
struct Segment {
    index: u32,
    checkpoint: bool,
    records: u32,
    start: MediaTime,
    end: MediaTime,
    first_sequence: u64,
    last_sequence: u64,
    raw_bytes: u64,
    stored_bytes: u64,
    sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Completion {
    segments: u32,
    records: u64,
    duration: MediaTime,
}
