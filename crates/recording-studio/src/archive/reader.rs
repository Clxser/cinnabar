//! Checkpoint-based seeking over bounded, individually verified compressed segments.

use std::{
    fs::{self, File},
    io::{self as std_io, Read},
    path::{Path, PathBuf},
};

use flate2::read::ZlibDecoder;
use sha2::{Digest, Sha256};

use super::{
    ArchiveError, COMPLETE_FILE, Completion, INDEX_FILE, MAX_METADATA_BYTES, MAX_RECORD_BYTES,
    MAX_SEGMENTS, RecordKind, RecordingInfo, Segment,
    format::{self, INDEX_ENTRY_BYTES, INDEX_MAGIC, RECORD_HEADER_BYTES},
    io,
};
use crate::MediaTime;

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
            self.visit_segment(segment, at, &cancelled, &mut payload, &mut visit)?;
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

    /// Bounded record buffers and streamed checksum validation prevent compressed allocation bombs.
    fn visit_segment(
        &self,
        segment: &Segment,
        at: MediaTime,
        cancelled: &impl Fn() -> bool,
        payload: &mut Vec<u8>,
        visit: &mut impl FnMut(ReplayRecord<'_>) -> Result<(), ArchiveError>,
    ) -> Result<(), ArchiveError> {
        let path = format::segment_path(&self.root, segment.index, false);
        let metadata = fs::symlink_metadata(&path).map_err(|error| io(&path, error))?;
        if !metadata.is_file() || metadata.len() != segment.stored_bytes {
            return Err(ArchiveError::Integrity(segment.index));
        }
        let file = File::open(&path).map_err(|error| io(&path, error))?;
        let mut decoded = ZlibDecoder::new(HashReader {
            file,
            digest: Sha256::new(),
            bytes: 0,
        });
        let mut previous = segment.start;
        let mut raw_bytes = 0u64;
        for index in 0..segment.records {
            if cancelled() {
                return Err(ArchiveError::Cancelled);
            }
            let mut header = [0; RECORD_HEADER_BYTES];
            decoded
                .read_exact(&mut header)
                .map_err(|error| io(&path, error))?;
            let kind = RecordKind::try_from(header[0])?;
            let time = MediaTime(u64::from_le_bytes(
                header[1..9].try_into().expect("fixed timestamp"),
            ));
            let sequence = u64::from_le_bytes(header[9..17].try_into().expect("fixed sequence"));
            let length =
                u32::from_le_bytes(header[17..21].try_into().expect("fixed length")) as usize;
            raw_bytes += RECORD_HEADER_BYTES as u64 + length as u64;
            let expected_kind = if segment.checkpoint {
                Some(if index == 0 {
                    RecordKind::CheckpointStart
                } else if index == segment.records - 1 {
                    RecordKind::CheckpointEnd
                } else {
                    RecordKind::CheckpointPart
                })
            } else {
                None
            };
            if length > MAX_RECORD_BYTES
                || raw_bytes > segment.raw_bytes
                || sequence != segment.first_sequence + u64::from(index)
                || time < previous
                || time > segment.end
                || index == 0 && time != segment.start
                || index == segment.records - 1 && time != segment.end
                || expected_kind.is_some_and(|expected| kind != expected)
                || !segment.checkpoint
                    && matches!(
                        kind,
                        RecordKind::CheckpointStart
                            | RecordKind::CheckpointPart
                            | RecordKind::CheckpointEnd
                    )
                || matches!(
                    kind,
                    RecordKind::CheckpointStart | RecordKind::CheckpointEnd
                ) && length != 0
            {
                return Err(ArchiveError::Integrity(segment.index));
            }
            payload.resize(length, 0);
            decoded
                .read_exact(payload)
                .map_err(|error| io(&path, error))?;
            if time <= at {
                visit(ReplayRecord {
                    sequence,
                    at: time,
                    kind,
                    payload,
                })?;
            }
            previous = time;
        }
        let mut trailing = [0];
        if decoded
            .read(&mut trailing)
            .map_err(|error| io(&path, error))?
            != 0
            || decoded.total_in() != segment.stored_bytes
            || decoded.total_out() != segment.raw_bytes
            || raw_bytes != segment.raw_bytes
            || decoded.get_ref().bytes != segment.stored_bytes
            || <[u8; 32]>::from(decoded.get_ref().digest.clone().finalize()) != segment.sha256
        {
            return Err(ArchiveError::Integrity(segment.index));
        }
        Ok(())
    }
}

struct HashReader {
    file: File,
    digest: Sha256,
    bytes: u64,
}

impl Read for HashReader {
    /// Hashes stored bytes as the decoder consumes the file, without a second disk pass.
    fn read(&mut self, out: &mut [u8]) -> std_io::Result<usize> {
        let count = self.file.read(out)?;
        self.digest.update(&out[..count]);
        self.bytes += count as u64;
        Ok(count)
    }
}
