//! Complete segment admission precedes callbacks; large checkpoints spill to anonymous scratch files.

use std::{
    fs::{self, File},
    io::{self as std_io, BufReader, BufWriter, Cursor, Read, Seek, Write},
    path::{Path, PathBuf},
};

use flate2::read::ZlibDecoder;
use sha2::{Digest, Sha256};

use super::{ArchiveError, MediaTime, RecordKind, ReplayRecord, Segment, format, io};
use crate::archive::{MAX_RECORD_BYTES, format::RECORD_HEADER_BYTES};

const MEMORY_BYTES: usize = 2 * MAX_RECORD_BYTES;
const COPY_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy)]
struct Header {
    kind: RecordKind,
    at: MediaTime,
    sequence: u64,
    length: usize,
}

impl Header {
    /// Fixed field decoding is shared by full admission and sequential playback of the admitted bytes.
    fn decode(bytes: [u8; RECORD_HEADER_BYTES]) -> Result<Self, ArchiveError> {
        Ok(Self {
            kind: RecordKind::try_from(bytes[0])?,
            at: MediaTime(u64::from_le_bytes(
                bytes[1..9].try_into().expect("fixed timestamp"),
            )),
            sequence: u64::from_le_bytes(bytes[9..17].try_into().expect("fixed sequence")),
            length: u32::from_le_bytes(bytes[17..21].try_into().expect("fixed length")) as usize,
        })
    }

    /// Index framing, sequence and timestamp bounds remain prerequisites even for skipped future records.
    fn admit(
        self,
        segment: Segment,
        index: u32,
        previous: MediaTime,
        raw_bytes: u64,
    ) -> Result<(), ArchiveError> {
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
        if self.length > MAX_RECORD_BYTES
            || raw_bytes > segment.raw_bytes
            || segment.first_sequence.checked_add(u64::from(index)) != Some(self.sequence)
            || self.at < previous
            || self.at > segment.end
            || index == 0 && self.at != segment.start
            || index == segment.records - 1 && self.at != segment.end
            || expected_kind.is_some_and(|expected| self.kind != expected)
            || !segment.checkpoint
                && matches!(
                    self.kind,
                    RecordKind::CheckpointStart
                        | RecordKind::CheckpointPart
                        | RecordKind::CheckpointEnd
                )
            || matches!(
                self.kind,
                RecordKind::CheckpointStart | RecordKind::CheckpointEnd
            ) && self.length != 0
        {
            return Err(ArchiveError::Integrity(segment.index));
        }
        Ok(())
    }
}

enum Storage {
    Memory(Cursor<Vec<u8>>),
    Scratch(BufReader<File>),
}
impl Read for Storage {
    fn read(&mut self, out: &mut [u8]) -> std_io::Result<usize> {
        match self {
            Self::Memory(input) => input.read(out),
            Self::Scratch(input) => input.read(out),
        }
    }
}

pub(super) struct VerifiedSegment {
    source: Storage,
    remaining: u32,
    pending: Option<Header>,
    path: PathBuf,
}

impl VerifiedSegment {
    /// Verifies all records and the full compressed checksum before making any payload available.
    pub fn load(
        root: &Path,
        segment: Segment,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, ArchiveError> {
        let path = format::segment_path(root, segment.index, false);
        let result = Self::load_inner(&path, segment, cancelled);
        if cancelled() {
            return Err(ArchiveError::Cancelled);
        }
        result
    }

    /// Small segments stay in memory; large baselines use a delete-on-close scratch file instead.
    fn load_inner(
        path: &Path,
        segment: Segment,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, ArchiveError> {
        let metadata = fs::symlink_metadata(path).map_err(|error| io(path, error))?;
        if !metadata.is_file() || metadata.len() != segment.stored_bytes {
            return Err(ArchiveError::Integrity(segment.index));
        }
        let file = File::open(path).map_err(|error| io(path, error))?;
        let mut decoded = ZlibDecoder::new(HashReader {
            file,
            digest: Sha256::new(),
            bytes: 0,
            cancelled,
        });
        let mut storage = Staging::new(segment.raw_bytes).map_err(|error| io(path, error))?;
        let mut previous = segment.start;
        let mut raw_bytes = 0_u64;
        let mut buffer = [0_u8; COPY_BYTES];
        for index in 0..segment.records {
            if cancelled() {
                return Err(ArchiveError::Cancelled);
            }
            let mut bytes = [0; RECORD_HEADER_BYTES];
            decoded
                .read_exact(&mut bytes)
                .map_err(|error| io(path, error))?;
            let header = Header::decode(bytes)?;
            raw_bytes = raw_bytes
                .checked_add(RECORD_HEADER_BYTES as u64 + header.length as u64)
                .ok_or(ArchiveError::Integrity(segment.index))?;
            header.admit(segment, index, previous, raw_bytes)?;
            storage.write_all(&bytes).map_err(|error| io(path, error))?;
            let mut remaining = header.length;
            while remaining != 0 {
                if cancelled() {
                    return Err(ArchiveError::Cancelled);
                }
                let count = remaining.min(buffer.len());
                decoded
                    .read_exact(&mut buffer[..count])
                    .map_err(|error| io(path, error))?;
                storage
                    .write_all(&buffer[..count])
                    .map_err(|error| io(path, error))?;
                remaining -= count;
            }
            previous = header.at;
        }
        let mut trailing = [0];
        if decoded
            .read(&mut trailing)
            .map_err(|error| io(path, error))?
            != 0
            || decoded.total_in() != segment.stored_bytes
            || decoded.total_out() != segment.raw_bytes
            || raw_bytes != segment.raw_bytes
            || decoded.get_ref().bytes != segment.stored_bytes
            || <[u8; 32]>::from(decoded.get_ref().digest.clone().finalize()) != segment.sha256
        {
            return Err(ArchiveError::Integrity(segment.index));
        }
        Ok(Self {
            source: storage.finish().map_err(|error| io(path, error))?,
            remaining: segment.records,
            pending: None,
            path: path.to_path_buf(),
        })
    }

    /// Retains one future header and one reusable payload, so advancing repeats neither IO nor callbacks.
    pub fn visit_to(
        &mut self,
        at: MediaTime,
        cancelled: &impl Fn() -> bool,
        payload: &mut Vec<u8>,
        visit: &mut impl FnMut(ReplayRecord<'_>) -> Result<(), ArchiveError>,
    ) -> Result<bool, ArchiveError> {
        while self.remaining != 0 {
            if cancelled() {
                return Err(ArchiveError::Cancelled);
            }
            let header = match self.pending.take() {
                Some(header) => header,
                None => {
                    let mut bytes = [0; RECORD_HEADER_BYTES];
                    self.source
                        .read_exact(&mut bytes)
                        .map_err(|error| io(&self.path, error))?;
                    Header::decode(bytes)?
                }
            };
            if header.at > at {
                self.pending = Some(header);
                return Ok(false);
            }
            payload.resize(header.length, 0);
            for chunk in payload.chunks_mut(COPY_BYTES) {
                if cancelled() {
                    return Err(ArchiveError::Cancelled);
                }
                self.source
                    .read_exact(chunk)
                    .map_err(|error| io(&self.path, error))?;
            }
            visit(ReplayRecord {
                sequence: header.sequence,
                at: header.at,
                kind: header.kind,
                payload,
            })?;
            self.remaining -= 1;
        }
        Ok(true)
    }

    /// Peeks one admitted timestamp while retaining its header for the next payload visit.
    pub fn next_time(&mut self) -> Result<Option<MediaTime>, ArchiveError> {
        if self.remaining == 0 {
            return Ok(None);
        }
        if self.pending.is_none() {
            let mut bytes = [0; RECORD_HEADER_BYTES];
            self.source
                .read_exact(&mut bytes)
                .map_err(|error| io(&self.path, error))?;
            self.pending = Some(Header::decode(bytes)?);
        }
        Ok(self.pending.map(|header| header.at))
    }
}

enum Staging {
    Memory(Vec<u8>),
    Scratch(BufWriter<File>),
}
impl Staging {
    /// The index's admitted raw-byte bound determines memory use before decompression starts.
    fn new(raw_bytes: u64) -> std_io::Result<Self> {
        if raw_bytes <= MEMORY_BYTES as u64 {
            Ok(Self::Memory(Vec::with_capacity(raw_bytes as usize)))
        } else {
            tempfile::tempfile()
                .map(|file| Self::Scratch(BufWriter::with_capacity(COPY_BYTES, file)))
        }
    }

    /// Scratch data needs no durable sync: it is verified temporary playback state, never a recording.
    fn finish(self) -> std_io::Result<Storage> {
        match self {
            Self::Memory(bytes) => Ok(Storage::Memory(Cursor::new(bytes))),
            Self::Scratch(output) => {
                let mut file = output.into_inner().map_err(|error| error.into_error())?;
                file.rewind()?;
                Ok(Storage::Scratch(BufReader::with_capacity(COPY_BYTES, file)))
            }
        }
    }
}
impl Write for Staging {
    fn write(&mut self, bytes: &[u8]) -> std_io::Result<usize> {
        match self {
            Self::Memory(out) => out.write(bytes),
            Self::Scratch(out) => out.write(bytes),
        }
    }
    fn flush(&mut self) -> std_io::Result<()> {
        match self {
            Self::Memory(out) => out.flush(),
            Self::Scratch(out) => out.flush(),
        }
    }
}

struct HashReader<'a> {
    file: File,
    digest: Sha256,
    bytes: u64,
    cancelled: &'a dyn Fn() -> bool,
}
impl Read for HashReader<'_> {
    /// Cancellation bounds compressed IO too; Interrupted would make read_exact retry indefinitely.
    fn read(&mut self, out: &mut [u8]) -> std_io::Result<usize> {
        if (self.cancelled)() {
            return Err(std_io::Error::other("recording read cancelled"));
        }
        let maximum = out.len().min(COPY_BYTES);
        let count = self.file.read(&mut out[..maximum])?;
        self.digest.update(&out[..count]);
        self.bytes += count as u64;
        Ok(count)
    }
}
