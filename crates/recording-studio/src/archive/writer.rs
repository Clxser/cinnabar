//! Streaming compression and append-only index publication, confined to the recording worker.

use std::{
    fs::{self, File},
    io::{self as std_io, Write},
    path::{Path, PathBuf},
};

use flate2::{Compression, write::ZlibEncoder};
use sha2::{Digest, Sha256};

use super::{
    ArchiveError, COMPLETE_FILE, Completion, INDEX_FILE, MAX_CHECKPOINT_BYTES, MAX_METADATA_BYTES,
    MAX_SEGMENTS, METADATA_FILE, RecordInput, RecordKind, RecordingConfig, RecordingInfo, Segment,
    format::{self, INDEX_ENTRY_BYTES, INDEX_MAGIC, RECORD_HEADER_BYTES},
    io,
};
use crate::MediaTime;

/// Only complete compressed segments enter the index. A process interruption leaves a valid prefix.
pub(super) struct Writer {
    root: PathBuf,
    index: File,
    config: RecordingConfig,
    open: Option<OpenSegment>,
    committed_bytes: u64,
    committed: Completion,
    next_sequence: u64,
    last_at: MediaTime,
    have_checkpoint: bool,
}

impl Writer {
    /// Creates a fresh recording directory; an existing recording is never appended or overwritten.
    pub(super) fn create(
        root: PathBuf,
        info: &RecordingInfo,
        config: RecordingConfig,
    ) -> Result<Self, ArchiveError> {
        info.validate()?;
        config.validate()?;
        fs::create_dir(&root).map_err(|error| io(&root, error))?;
        let metadata_bytes = format::write_metadata(&root.join(METADATA_FILE), info)?;
        let index_path = root.join(INDEX_FILE);
        let mut index = File::options()
            .write(true)
            .create_new(true)
            .open(&index_path)
            .map_err(|error| io(&index_path, error))?;
        index
            .write_all(INDEX_MAGIC)
            .and_then(|()| index.sync_all())
            .map_err(|error| io(&index_path, error))?;
        format::sync_directory(&root)?;
        Ok(Self {
            root,
            index,
            config,
            open: None,
            committed_bytes: metadata_bytes + INDEX_MAGIC.len() as u64,
            committed: Completion {
                segments: 0,
                records: 0,
                duration: MediaTime::ZERO,
            },
            next_sequence: 1,
            last_at: MediaTime::ZERO,
            have_checkpoint: false,
        })
    }

    /// Appends one ordered record without copying its payload into an intermediate segment buffer.
    pub(super) fn append(
        &mut self,
        sequence: u64,
        record: &RecordInput,
    ) -> Result<(), ArchiveError> {
        record.validate()?;
        if sequence != self.next_sequence || record.at < self.last_at {
            return Err(ArchiveError::Invalid(
                "replay records must retain timestamp and sequence order",
            ));
        }
        let inside_checkpoint = self.open.as_ref().is_some_and(|segment| segment.checkpoint);
        match record.kind {
            RecordKind::CheckpointStart => {
                if inside_checkpoint || !self.have_checkpoint && record.at != MediaTime::ZERO {
                    return Err(ArchiveError::Invalid(
                        "the initial checkpoint must begin at recording time zero",
                    ));
                }
                self.commit_open()?;
                self.begin(true, record.at, sequence)?;
            }
            RecordKind::CheckpointPart | RecordKind::CheckpointEnd => {
                if !inside_checkpoint
                    || self
                        .open
                        .as_ref()
                        .is_none_or(|segment| segment.start != record.at)
                {
                    return Err(ArchiveError::Invalid(
                        "checkpoint parts must be enclosed at one timestamp",
                    ));
                }
            }
            _ => {
                if inside_checkpoint || !self.have_checkpoint {
                    return Err(ArchiveError::Invalid(
                        "scene records require a completed initial checkpoint",
                    ));
                }
                let bytes = RECORD_HEADER_BYTES as u64 + record.payload.len() as u64;
                if self.open.as_ref().is_some_and(|segment| {
                    segment.raw_bytes + bytes > self.config.segment_bytes
                        || record.at.saturating_sub(segment.start) >= self.config.segment_duration
                }) {
                    self.commit_open()?;
                }
                if self.open.is_none() {
                    self.begin(false, record.at, sequence)?;
                }
            }
        }
        self.open
            .as_mut()
            .ok_or(ArchiveError::Invalid("replay has no active segment"))?
            .append(sequence, record)?;
        self.next_sequence = sequence
            .checked_add(1)
            .ok_or(ArchiveError::Invalid("replay sequence exhausted"))?;
        self.last_at = record.at;
        if record.kind == RecordKind::CheckpointEnd {
            self.commit_open()?;
            self.have_checkpoint = true;
        }
        Ok(())
    }

    /// Publishes a normal stop only after the last scene segment is durable.
    pub(super) fn complete(mut self, at: MediaTime) -> Result<(u64, Completion), ArchiveError> {
        if !self.have_checkpoint
            || self.open.as_ref().is_some_and(|segment| segment.checkpoint)
            || at < self.last_at
            || at.0 > crate::time::MAX_MEDIA_MICROS
        {
            return Err(ArchiveError::Invalid(
                "recording cannot stop inside a checkpoint or before its last record",
            ));
        }
        self.commit_open()?;
        self.committed.duration = at;
        self.committed_bytes +=
            format::write_metadata(&self.root.join(COMPLETE_FILE), &self.committed)?;
        Ok((self.committed_bytes, self.committed))
    }

    /// Keeps the last valid scene segment after producer backpressure; partial checkpoints stay unindexed.
    pub(super) fn interrupt(mut self) -> Result<(u64, Completion), ArchiveError> {
        if self
            .open
            .as_ref()
            .is_some_and(|segment| !segment.checkpoint)
        {
            self.commit_open()?;
        }
        Ok((self.committed_bytes, self.committed))
    }

    pub(super) fn progress(&self) -> (u64, Completion) {
        (self.committed_bytes, self.committed)
    }

    /// Reserves a numbered staging file and a strict remaining disk budget before compression begins.
    fn begin(
        &mut self,
        checkpoint: bool,
        at: MediaTime,
        sequence: u64,
    ) -> Result<(), ArchiveError> {
        if self.committed.segments as usize >= MAX_SEGMENTS {
            return Err(ArchiveError::Invalid("recording segment limit reached"));
        }
        let reserve = INDEX_ENTRY_BYTES as u64 + MAX_METADATA_BYTES as u64;
        let remaining = self
            .config
            .disk_quota
            .checked_sub(self.committed_bytes + reserve)
            .filter(|&remaining| remaining > 0)
            .ok_or(ArchiveError::Quota)?;
        self.open = Some(OpenSegment::create(
            &self.root,
            self.committed.segments,
            checkpoint,
            at,
            sequence,
            remaining.min(MAX_CHECKPOINT_BYTES * 2),
            self.config.compression_level,
        )?);
        Ok(())
    }

    /// Renames the finished segment before appending its checksummed immutable index entry.
    fn commit_open(&mut self) -> Result<(), ArchiveError> {
        let Some(segment) = self.open.take() else {
            return Ok(());
        };
        let segment = segment.finish(&self.root)?;
        let path = self.root.join(INDEX_FILE);
        self.index
            .write_all(&segment.encode())
            .and_then(|()| self.index.sync_data())
            .map_err(|error| io(&path, error))?;
        self.committed_bytes += segment.stored_bytes + INDEX_ENTRY_BYTES as u64;
        self.committed.segments += 1;
        self.committed.records += u64::from(segment.records);
        self.committed.duration = segment.end;
        Ok(())
    }
}

struct OpenSegment {
    index: u32,
    checkpoint: bool,
    start: MediaTime,
    end: MediaTime,
    first_sequence: u64,
    last_sequence: u64,
    records: u32,
    raw_bytes: u64,
    path: PathBuf,
    compressed: ZlibEncoder<HashWriter>,
}

impl OpenSegment {
    /// Compression is streaming, including multipart world snapshots that exceed one queue message.
    fn create(
        root: &Path,
        index: u32,
        checkpoint: bool,
        at: MediaTime,
        sequence: u64,
        maximum: u64,
        compression_level: u32,
    ) -> Result<Self, ArchiveError> {
        let path = format::segment_path(root, index, true);
        let file = File::options()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|error| io(&path, error))?;
        Ok(Self {
            index,
            checkpoint,
            start: at,
            end: at,
            first_sequence: sequence,
            last_sequence: sequence,
            records: 0,
            raw_bytes: 0,
            path,
            compressed: ZlibEncoder::new(
                HashWriter {
                    file,
                    digest: Sha256::new(),
                    bytes: 0,
                    maximum,
                },
                Compression::new(compression_level),
            ),
        })
    }

    /// Writes fixed framing followed by borrowed bytes; oversized checkpoints stop explicitly.
    fn append(&mut self, sequence: u64, record: &RecordInput) -> Result<(), ArchiveError> {
        let total = self.raw_bytes + RECORD_HEADER_BYTES as u64 + record.payload.len() as u64;
        if total > MAX_CHECKPOINT_BYTES {
            return Err(ArchiveError::Invalid(
                "replay checkpoint or segment exceeds its byte limit",
            ));
        }
        let mut header = [0; RECORD_HEADER_BYTES];
        header[0] = record.kind as u8;
        header[1..9].copy_from_slice(&record.at.0.to_le_bytes());
        header[9..17].copy_from_slice(&sequence.to_le_bytes());
        header[17..21].copy_from_slice(&(record.payload.len() as u32).to_le_bytes());
        self.compressed
            .write_all(&header)
            .and_then(|()| self.compressed.write_all(&record.payload))
            .map_err(|error| io(&self.path, error))?;
        self.records += 1;
        self.raw_bytes = total;
        self.end = record.at;
        self.last_sequence = sequence;
        Ok(())
    }

    /// A completed zlib stream and synced file are prerequisites for index publication.
    fn finish(self, root: &Path) -> Result<Segment, ArchiveError> {
        let mut compressed = self
            .compressed
            .finish()
            .map_err(|error| io(&self.path, error))?;
        compressed
            .flush()
            .and_then(|()| compressed.file.sync_all())
            .map_err(|error| io(&self.path, error))?;
        let stored_bytes = compressed.bytes;
        let sha256 = compressed.digest.finalize().into();
        drop(compressed.file);
        let target = format::segment_path(root, self.index, false);
        fs::rename(&self.path, &target).map_err(|error| io(&target, error))?;
        format::sync_directory(root)?;
        Ok(Segment {
            index: self.index,
            checkpoint: self.checkpoint,
            records: self.records,
            start: self.start,
            end: self.end,
            first_sequence: self.first_sequence,
            last_sequence: self.last_sequence,
            raw_bytes: self.raw_bytes,
            stored_bytes,
            sha256,
        })
    }
}

struct HashWriter {
    file: File,
    digest: Sha256,
    bytes: u64,
    maximum: u64,
}

impl Write for HashWriter {
    /// Counts only successfully written bytes and enforces quota even while a zlib stream is open.
    fn write(&mut self, bytes: &[u8]) -> std_io::Result<usize> {
        if bytes.len() as u64 > self.maximum.saturating_sub(self.bytes) {
            return Err(std_io::Error::other("recording storage quota reached"));
        }
        let count = self.file.write(bytes)?;
        self.digest.update(&bytes[..count]);
        self.bytes += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> std_io::Result<()> {
        self.file.flush()
    }
}
