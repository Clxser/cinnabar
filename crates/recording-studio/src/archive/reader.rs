//! Checkpoint-based seeking over bounded, individually verified compressed segments.

use std::{
    fs, io as std_io,
    path::{Path, PathBuf},
};

use super::{
    ArchiveError, COMPLETE_FILE, Completion, INDEX_FILE, MAX_METADATA_BYTES, MAX_SEGMENTS,
    RecordKind, RecordingInfo, Segment,
    format::{self, INDEX_ENTRY_BYTES, INDEX_MAGIC},
    io,
};
use crate::MediaTime;

mod cursor;
mod segment;
pub use cursor::RecordingCursor;
use segment::VerifiedSegment;

#[derive(Clone, Copy, Debug, Default)]
pub struct Recovery {
    /// An incomplete recording remains usable up to its last committed segment.
    pub interrupted: bool,
    pub discarded_index_bytes: usize,
}

/// Borrowed bytes remain valid only for this visitor call; the scene owner admits their own schema.
#[derive(Clone, Copy, Debug)]
pub struct ReplayRecord<'a> {
    pub sequence: u64,
    pub at: MediaTime,
    pub kind: RecordKind,
    pub payload: &'a [u8],
}

pub struct RecordingReader {
    root: PathBuf,
    info: RecordingInfo,
    segments: Vec<Segment>,
    checkpoints: Vec<usize>,
    duration: MediaTime,
    recovery: Recovery,
}

impl RecordingReader {
    /// Loads the small index only; opening a library item does not deserialize its world or audio.
    pub fn open(root: &Path) -> Result<Self, ArchiveError> {
        let info = RecordingInfo::read(root)?;
        let index = format::read_bounded(
            &root.join(INDEX_FILE),
            INDEX_MAGIC.len() + MAX_SEGMENTS * INDEX_ENTRY_BYTES,
        )?;
        if !index.starts_with(INDEX_MAGIC) {
            return Err(ArchiveError::Invalid(
                "recording index has an unsupported header",
            ));
        }
        let completion_path = root.join(COMPLETE_FILE);
        let completion: Option<Completion> = match fs::symlink_metadata(&completion_path) {
            Ok(_) => Some(serde_json::from_slice(&format::read_bounded(
                &completion_path,
                MAX_METADATA_BYTES,
            )?)?),
            Err(error) if error.kind() == std_io::ErrorKind::NotFound => None,
            Err(error) => return Err(io(&completion_path, error)),
        };
        let body = &index[INDEX_MAGIC.len()..];
        let discarded_index_bytes = body.len() % INDEX_ENTRY_BYTES;
        if completion.is_some() && discarded_index_bytes != 0 {
            return Err(ArchiveError::Invalid(
                "completed recording has a truncated index",
            ));
        }
        let mut segments: Vec<Segment> = Vec::with_capacity(body.len() / INDEX_ENTRY_BYTES);
        let mut checkpoints = Vec::new();
        for bytes in body.chunks_exact(INDEX_ENTRY_BYTES) {
            let segment = Segment::decode(bytes.try_into().expect("fixed index entry"))?;
            if segment.index as usize != segments.len()
                || segments.last().is_some_and(|previous| {
                    Some(segment.first_sequence) != previous.last_sequence.checked_add(1)
                        || segment.start < previous.end
                })
                || segments.is_empty()
                    && (!segment.checkpoint
                        || segment.start != MediaTime::ZERO
                        || segment.first_sequence != 1)
            {
                return Err(ArchiveError::Invalid(
                    "recording index is not contiguous and checkpointed",
                ));
            }
            if segment.checkpoint {
                checkpoints.push(segments.len());
            }
            segments.push(segment);
        }
        let last = segments.last().ok_or(ArchiveError::Invalid(
            "recording has no completed initial checkpoint",
        ))?;
        let duration = if let Some(completion) = completion {
            if completion.segments as usize != segments.len()
                || completion.records != last.last_sequence
                || completion.duration < last.end
                || completion.duration.0 > crate::time::MAX_MEDIA_MICROS
            {
                return Err(ArchiveError::Invalid(
                    "recording completion disagrees with its committed index",
                ));
            }
            completion.duration
        } else {
            last.end
        };
        Ok(Self {
            root: root.to_path_buf(),
            info,
            segments,
            checkpoints,
            duration,
            recovery: Recovery {
                interrupted: completion.is_none(),
                discarded_index_bytes,
            },
        })
    }

    pub fn info(&self) -> &RecordingInfo {
        &self.info
    }
    pub const fn duration(&self) -> MediaTime {
        self.duration
    }
    pub const fn recovery(&self) -> Recovery {
        self.recovery
    }

    /// Moves the validated index into a worker-owned cursor for incremental forward playback.
    pub fn into_cursor(self) -> RecordingCursor {
        RecordingCursor::new(self)
    }

    /// The checkpoint preceding a seek is found with binary search, independently of recording length.
    pub fn checkpoint_at(&self, at: MediaTime) -> Result<MediaTime, ArchiveError> {
        let index = self.checkpoint_index(at)?;
        Ok(self.segments[index].start)
    }

    /// Reconstructs a staging scene from its nearest complete checkpoint, then applies ordered deltas.
    /// The caller publishes that scene only on success: corrupt data or a newer seek discards the draft.
    /// Running this on a worker keeps decompression and large snapshots off the render/input thread.
    pub fn visit_to(
        &self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
        mut visit: impl FnMut(ReplayRecord<'_>) -> Result<(), ArchiveError>,
    ) -> Result<(), ArchiveError> {
        let checkpoint = self.checkpoint_index(at)?;
        let mut payload = Vec::new();
        for segment in &self.segments[checkpoint..] {
            if segment.start > at {
                break;
            }
            if cancelled() {
                return Err(ArchiveError::Cancelled);
            }
            let mut verified = VerifiedSegment::load(&self.root, *segment, &cancelled)?;
            verified.visit_to(at, &cancelled, &mut payload, &mut visit)?;
        }
        if cancelled() {
            return Err(ArchiveError::Cancelled);
        }
        Ok(())
    }

    /// Refuses out-of-source seeks before any file is opened or caller state is changed.
    fn checkpoint_index(&self, at: MediaTime) -> Result<usize, ArchiveError> {
        if at > self.duration {
            return Err(ArchiveError::Invalid("seek exceeds the recording duration"));
        }
        let next = self
            .checkpoints
            .partition_point(|&index| self.segments[index].start <= at);
        self.checkpoints
            .get(next.saturating_sub(1))
            .copied()
            .ok_or(ArchiveError::Invalid(
                "seek has no completed world checkpoint",
            ))
    }
}
