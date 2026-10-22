//! Immutable, hash-addressed recording assets. All filesystem work belongs on capture/seek workers.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::archive::ArchiveError;

const DIRECTORY: &str = "objects";
const EXTENSION: &str = "cinasset";
const MAX_SHARED_PIXELS: usize = 4096;
pub const MAX_ASSET_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_RECORDING_ASSETS: usize = 65_536;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AssetId(pub [u8; 32]);

impl std::fmt::Display for AssetId {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(out, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct AssetReader {
    directory: PathBuf,
}

impl AssetReader {
    /// Resolves only fixed digest filenames beneath this recording's immutable object directory.
    pub fn new(recording: &Path) -> Self {
        Self {
            directory: recording.join(DIRECTORY),
        }
    }

    /// No decoded value escapes until its entire bounded source and SHA-256 have been verified.
    /// A decoder must consume its whole object; trailing bytes are rejected rather than ignored.
    pub fn read<T>(
        &self,
        id: AssetId,
        maximum: u64,
        decode: impl FnOnce(&mut dyn Read) -> Result<T, ArchiveError>,
    ) -> Result<T, ArchiveError> {
        self.read_cancellable(id, maximum, || false, decode)
    }

    /// Newer seeks can cancel large texture reads between bounded IO chunks, before scene publication.
    pub fn read_cancellable<T>(
        &self,
        id: AssetId,
        maximum: u64,
        cancelled: impl Fn() -> bool,
        decode: impl FnOnce(&mut dyn Read) -> Result<T, ArchiveError>,
    ) -> Result<T, ArchiveError> {
        if cancelled() {
            return Err(ArchiveError::Cancelled);
        }
        let path = self.path(id);
        let metadata = fs::symlink_metadata(&path).map_err(|error| file_error(&path, error))?;
        if !metadata.is_file() || metadata.len() > maximum.min(MAX_ASSET_BYTES) {
            return Err(ArchiveError::Invalid(
                "recording asset exceeds its type or byte bounds",
            ));
        }
        let file = File::open(&path).map_err(|error| file_error(&path, error))?;
        let mut reader = HashReader {
            input: BufReader::new(file),
            hash: Sha256::new(),
            remaining: metadata.len(),
            path: &path,
            cancelled: &cancelled,
        };
        let value = decode(&mut reader);
        if cancelled() {
            return Err(ArchiveError::Cancelled);
        }
        let value = value?;
        let mut trailing = [0_u8; 1];
        let end = reader.read(&mut trailing);
        if cancelled() {
            return Err(ArchiveError::Cancelled);
        }
        if end.map_err(|error| file_error(&path, error))? != 0 || reader.remaining != 0 {
            return Err(ArchiveError::Invalid(
                "recording asset decoder left trailing bytes",
            ));
        }
        let actual: [u8; 32] = reader.hash.finalize().into();
        if actual != id.0 {
            return Err(ArchiveError::Invalid("recording asset SHA-256 mismatch"));
        }
        Ok(value)
    }

    /// Digest bytes, never supplied paths, determine the object filename.
    fn path(&self, id: AssetId) -> PathBuf {
        self.directory.join(format!("{id}.{EXTENSION}"))
    }
}

pub struct AssetWriter {
    reader: AssetReader,
    maximum: u64,
    stored: u64,
    objects: usize,
    shared_pixels: BTreeMap<usize, (Weak<[u8]>, AssetId)>,
    disk: crate::disk::DiskBudget,
}

impl AssetWriter {
    /// Admits an existing bounded store without following symlinks or charging unfinished staging files.
    pub fn open(recording: &Path, maximum: u64) -> Result<Self, ArchiveError> {
        Self::open_with_disk(recording, maximum, crate::disk::DiskBudget::new(maximum))
    }

    /// Each store has one worker owner; existing bytes are adopted once into the shared take allowance.
    pub fn open_with_disk(
        recording: &Path,
        maximum: u64,
        disk: crate::disk::DiskBudget,
    ) -> Result<Self, ArchiveError> {
        if maximum == 0 {
            return Err(ArchiveError::Invalid("recording asset quota is zero"));
        }
        let reader = AssetReader::new(recording);
        match fs::create_dir(&reader.directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if !fs::symlink_metadata(&reader.directory)
                    .map_err(|error| file_error(&reader.directory, error))?
                    .is_dir()
                {
                    return Err(ArchiveError::Invalid(
                        "recording object directory is not a directory",
                    ));
                }
            }
            Err(error) => return Err(file_error(&reader.directory, error)),
        }
        let mut stored = 0_u64;
        let mut objects = 0;
        for (index, entry) in fs::read_dir(&reader.directory)
            .map_err(|error| file_error(&reader.directory, error))?
            .enumerate()
        {
            if index > MAX_RECORDING_ASSETS * 2 {
                return Err(ArchiveError::Quota);
            }
            let entry = entry.map_err(|error| file_error(&reader.directory, error))?;
            let path = entry.path();
            if path
                .extension()
                .is_none_or(|extension| extension != EXTENSION)
            {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if stem.len() != 64
                || !stem
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err(ArchiveError::Invalid("invalid recording object filename"));
            }
            let metadata = fs::symlink_metadata(&path).map_err(|error| file_error(&path, error))?;
            if !metadata.is_file() || metadata.len() > MAX_ASSET_BYTES {
                return Err(ArchiveError::Invalid("invalid recording object file"));
            }
            stored = stored
                .checked_add(metadata.len())
                .filter(|&bytes| bytes <= maximum)
                .ok_or(ArchiveError::Quota)?;
            objects += 1;
            if objects > MAX_RECORDING_ASSETS {
                return Err(ArchiveError::Quota);
            }
        }
        disk.adopt(stored)
            .map_err(|error| file_error(&reader.directory, error))?;
        Ok(Self {
            reader,
            maximum,
            stored,
            objects,
            shared_pixels: BTreeMap::new(),
            disk,
        })
    }

    pub const fn stored_bytes(&self) -> u64 {
        self.stored
    }

    /// Reuses an already published immutable pixel allocation without rehashing or staging its bytes.
    /// Weak leases guard pointer reuse; expired allocation handles are pruned before a cache miss writes.
    pub fn put_shared_pixels(&mut self, bytes: &Arc<[u8]>) -> Result<AssetId, ArchiveError> {
        if bytes.is_empty() {
            return Err(ArchiveError::Invalid("empty recorded pixel allocation"));
        }
        let key = Arc::as_ptr(bytes) as *const u8 as usize;
        if let Some((old, id)) = self.shared_pixels.get(&key)
            && old.upgrade().is_some_and(|old| Arc::ptr_eq(&old, bytes))
        {
            return Ok(*id);
        }
        self.shared_pixels
            .retain(|_, (pixels, _)| pixels.strong_count() != 0);
        let directory = self.reader.directory.clone();
        let id = self.put(bytes.len() as u64, |out| {
            out.write_all(bytes)
                .map_err(|error| file_error(&directory, error))
        })?;
        if self.shared_pixels.len() < MAX_SHARED_PIXELS {
            self.shared_pixels.insert(key, (Arc::downgrade(bytes), id));
        }
        Ok(id)
    }

    /// Streams one object through bounded hashing and atomic no-clobber publication.
    /// Immutable objects deduplicate by content, independently of allocator addresses or frame IDs.
    pub fn put(
        &mut self,
        maximum: u64,
        encode: impl FnOnce(&mut dyn Write) -> Result<(), ArchiveError>,
    ) -> Result<AssetId, ArchiveError> {
        if self.objects >= MAX_RECORDING_ASSETS {
            return Err(ArchiveError::Quota);
        }
        let limit = maximum
            .min(MAX_ASSET_BYTES)
            .min(self.maximum.saturating_sub(self.stored));
        let mut staged = crate::disk::StagedFile::new(
            &self.reader.directory,
            ".object-",
            ".part",
            self.disk.clone(),
        )
        .map_err(|error| file_error(&self.reader.directory, error))?;
        let (hash, bytes) = {
            let mut writer = HashWriter {
                output: BufWriter::new(
                    staged
                        .writer()
                        .map_err(|error| file_error(&self.reader.directory, error))?,
                ),
                hash: Sha256::new(),
                bytes: 0,
                maximum: limit,
                exceeded: false,
            };
            let encoded = encode(&mut writer);
            if writer.exceeded {
                return Err(ArchiveError::Quota);
            }
            encoded?;
            writer
                .flush()
                .map_err(|error| file_error(&self.reader.directory, error))?;
            let HashWriter {
                output,
                hash,
                bytes,
                ..
            } = writer;
            drop(output);
            (hash, bytes)
        };
        staged
            .sync_all()
            .map_err(|error| file_error(&self.reader.directory, error))?;
        let id = AssetId(hash.finalize().into());
        let destination = self.reader.path(id);
        match staged.persist_noclobber(&destination) {
            Ok(_) => {
                self.objects += 1;
                self.stored += bytes;
            }
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                self.reader.read(id, limit, |input| {
                    io::copy(input, &mut io::sink())
                        .map_err(|error| file_error(&destination, error))?;
                    Ok(())
                })?;
            }
            Err(error) => return Err(file_error(&destination, error.error)),
        }
        sync_directory(&self.reader.directory)?;
        Ok(id)
    }
}

struct HashWriter<W> {
    output: W,
    hash: Sha256,
    bytes: u64,
    maximum: u64,
    exceeded: bool,
}
impl<W: Write> Write for HashWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > self.maximum.saturating_sub(self.bytes) {
            self.exceeded = true;
            return Err(io::Error::other("recording asset quota reached"));
        }
        let count = self.output.write(bytes)?;
        self.hash.update(&bytes[..count]);
        self.bytes += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

struct HashReader<'a, R> {
    input: R,
    hash: Sha256,
    remaining: u64,
    path: &'a Path,
    cancelled: &'a dyn Fn() -> bool,
}
impl<R: Read> Read for HashReader<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if (self.cancelled)() {
            return Err(io::Error::other("recording asset read cancelled"));
        }
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut extra = [0_u8; 1];
            if self.input.read(&mut extra)? != 0 {
                return Err(io::Error::other("recording object changed during decoding"));
            }
            return Ok(0);
        }
        let maximum = buffer.len().min(self.remaining as usize).min(64 * 1024);
        let count = self.input.read(&mut buffer[..maximum])?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("{}: truncated recording asset", self.path.display()),
            ));
        }
        self.hash.update(&buffer[..count]);
        self.remaining -= count as u64;
        Ok(count)
    }
}

/// Syncs directory entries where supported; Windows persist_noclobber uses its native move primitive.
fn sync_directory(path: &Path) -> Result<(), ArchiveError> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| file_error(path, error))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Retains the actionable affected filename without logging unrelated account or environment data.
fn file_error(path: &Path, source: io::Error) -> ArchiveError {
    ArchiveError::Io {
        path: path.to_path_buf(),
        source,
    }
}
