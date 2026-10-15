//! Fixed framing shared by the recording writer and bounded replay reader.

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use super::{ArchiveError, MAX_CHECKPOINT_BYTES, MAX_METADATA_BYTES, Segment, io};
use crate::{MediaTime, time::MAX_MEDIA_MICROS};

pub(super) const INDEX_MAGIC: &[u8; 8] = b"CINRPL\x01\0";
pub(super) const INDEX_ENTRY_BYTES: usize = 96;
pub(super) const RECORD_HEADER_BYTES: usize = 21;

impl Segment {
    /// Encodes one index commit; its immutable segment must already be durable.
    pub(super) fn encode(self) -> [u8; INDEX_ENTRY_BYTES] {
        let mut out = [0; INDEX_ENTRY_BYTES];
        out[0..4].copy_from_slice(&self.index.to_le_bytes());
        out[4..8].copy_from_slice(&u32::from(self.checkpoint).to_le_bytes());
        out[8..12].copy_from_slice(&self.records.to_le_bytes());
        out[16..24].copy_from_slice(&self.start.0.to_le_bytes());
        out[24..32].copy_from_slice(&self.end.0.to_le_bytes());
        out[32..40].copy_from_slice(&self.first_sequence.to_le_bytes());
        out[40..48].copy_from_slice(&self.last_sequence.to_le_bytes());
        out[48..56].copy_from_slice(&self.raw_bytes.to_le_bytes());
        out[56..64].copy_from_slice(&self.stored_bytes.to_le_bytes());
        out[64..96].copy_from_slice(&self.sha256);
        out
    }

    /// Rejects implausible sizes and counters before a decoder allocates or opens segment data.
    pub(super) fn decode(bytes: &[u8; INDEX_ENTRY_BYTES]) -> Result<Self, ArchiveError> {
        let u32_at = |offset| {
            u32::from_le_bytes(
                bytes[offset..offset + 4]
                    .try_into()
                    .expect("fixed index field"),
            )
        };
        let u64_at = |offset| {
            u64::from_le_bytes(
                bytes[offset..offset + 8]
                    .try_into()
                    .expect("fixed index field"),
            )
        };
        let segment = Self {
            index: u32_at(0),
            checkpoint: u32_at(4) == 1,
            records: u32_at(8),
            start: MediaTime(u64_at(16)),
            end: MediaTime(u64_at(24)),
            first_sequence: u64_at(32),
            last_sequence: u64_at(40),
            raw_bytes: u64_at(48),
            stored_bytes: u64_at(56),
            sha256: bytes[64..96].try_into().expect("fixed digest field"),
        };
        if u32_at(4) > 1
            || u32_at(12) != 0
            || segment.records == 0
            || segment.first_sequence == 0
            || segment.last_sequence.checked_sub(segment.first_sequence)
                != Some(u64::from(segment.records) - 1)
            || segment.start > segment.end
            || segment.end.0 > MAX_MEDIA_MICROS
            || segment.checkpoint && (segment.records < 2 || segment.start != segment.end)
            || segment.raw_bytes < u64::from(segment.records) * RECORD_HEADER_BYTES as u64
            || segment.raw_bytes > MAX_CHECKPOINT_BYTES
            || segment.stored_bytes == 0
            || segment.stored_bytes > MAX_CHECKPOINT_BYTES * 2
        {
            return Err(ArchiveError::Invalid("invalid replay segment index"));
        }
        Ok(segment)
    }
}

/// Derives segment paths only from an admitted numeric index, never from archive-controlled text.
pub(super) fn segment_path(root: &Path, index: u32, partial: bool) -> PathBuf {
    root.join(format!(
        "{index:08}.{}",
        if partial { "part" } else { "cinseg" }
    ))
}

/// Reads only regular, bounded files; incomplete segment staging files are never considered commits.
pub(super) fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, ArchiveError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io(path, error))?;
    if !metadata.is_file() || metadata.len() > maximum as u64 {
        return Err(ArchiveError::Invalid(
            "recording metadata file exceeds its bounds",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .map_err(|error| io(path, error))?
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io(path, error))?;
    if bytes.len() > maximum {
        return Err(ArchiveError::Invalid(
            "recording metadata file grew beyond its bounds",
        ));
    }
    Ok(bytes)
}

/// Creates one small metadata file exclusively, keeping accidental path reuse from overwriting media.
pub(super) fn write_metadata<T: serde::Serialize>(
    path: &Path,
    value: &T,
) -> Result<u64, ArchiveError> {
    let bytes = serde_json::to_vec_pretty(value)?;
    if bytes.len() > MAX_METADATA_BYTES {
        return Err(ArchiveError::Invalid(
            "recording metadata exceeds its byte limit",
        ));
    }
    let mut file = File::options()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io(path, error))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io(path, error))?;
    sync_directory(
        path.parent()
            .ok_or(ArchiveError::Invalid("metadata has no parent directory"))?,
    )?;
    Ok(bytes.len() as u64)
}

/// Makes published directory entries durable where the platform exposes directory fsync.
pub(super) fn sync_directory(path: &Path) -> Result<(), ArchiveError> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| io(path, error))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
