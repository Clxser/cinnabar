//! Exclusive staging and atomic replacement preserve the last saved project on write failures.

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::{StorageError, io};
use crate::{Project, model::MAX_PROJECT_BYTES};

static SAVE_NONCE: AtomicU64 = AtomicU64::new(1);
const STAGING_ATTEMPTS: usize = 32;

/// Resolves a selected path once at the filesystem edge, rather than depending on future cwd changes.
pub(super) fn absolute(path: &Path) -> Result<PathBuf, StorageError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|base| base.join(path))
        .map_err(|source| io(path, source))
}

/// Loads bounded metadata only; missing media is relinked separately rather than destroying the project.
pub(super) fn load(path: &Path) -> Result<Project, StorageError> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|source| io(path, source))?
        .take(MAX_PROJECT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| io(path, source))?;
    Ok(Project::from_json(&bytes)?)
}

/// Saves a metadata copy with portable paths; the active editor and immutable media stay untouched.
pub(super) fn save(path: &Path, project: &Project, source_base: &Path) -> Result<(), StorageError> {
    let parent = path.parent().ok_or(StorageError::InvalidPath)?;
    let bytes = portable_bytes(parent, project, source_base)?;
    fs::create_dir_all(parent).map_err(|source| io(parent, source))?;
    let (staging, mut file) = create_staging(parent)?;
    let write_result = file
        .write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| io(&staging, source));
    drop(file);
    let result =
        write_result.and_then(|()| fs::rename(&staging, path).map_err(|source| io(path, source)));
    if result.is_err() {
        // Only this call's exclusively created file can be removed here.
        let _ = fs::remove_file(&staging);
    }
    result?;
    sync_parent(parent)
}

/// Publishes a new project without overwriting an unrelated file, even across simultaneous app sessions.
pub(super) fn save_new(
    parent: &Path,
    project: &Project,
    source_base: &Path,
) -> Result<PathBuf, StorageError> {
    let bytes = portable_bytes(parent, project, source_base)?;
    fs::create_dir_all(parent).map_err(|source| io(parent, source))?;
    let mut staging = tempfile::Builder::new()
        .prefix(".cinnabar-studio-")
        .suffix(".saving")
        .tempfile_in(parent)
        .map_err(|source| io(parent, source))?;
    staging
        .write_all(&bytes)
        .and_then(|()| staging.as_file().sync_all())
        .map_err(|source| io(staging.path(), source))?;
    for index in 1..=1024 {
        let path = parent.join(format!(
            "Studio project {index}.{}",
            crate::model::PROJECT_EXTENSION
        ));
        match staging.persist_noclobber(&path) {
            Ok(_) => {
                sync_parent(parent)?;
                return Ok(path);
            }
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                staging = error.file
            }
            Err(error) => return Err(io(&path, error.error)),
        }
    }
    Err(StorageError::NamesExhausted)
}

/// Rewrites paths only in the serialized copy, keeping Save As independent of the active editor.
fn portable_bytes(
    parent: &Path,
    project: &Project,
    source_base: &Path,
) -> Result<Vec<u8>, StorageError> {
    let mut portable = project.clone();
    for source in &mut portable.sources {
        let absolute = if source.path.is_absolute() {
            source.path.clone()
        } else {
            source_base.join(&source.path)
        };
        source.path = absolute
            .strip_prefix(parent)
            .map_or_else(|_| absolute.clone(), Path::to_path_buf);
    }
    Ok(portable.to_json()?)
}

/// Flushes the published directory entry on hosts exposing directory fsync.
fn sync_parent(parent: &Path) -> Result<(), StorageError> {
    #[cfg(unix)]
    File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|source| io(parent, source))?;
    #[cfg(not(unix))]
    let _ = parent;
    Ok(())
}

/// A stale staging file from an interrupted save never overwrites another operation's data.
fn create_staging(parent: &Path) -> Result<(PathBuf, File), StorageError> {
    for _ in 0..STAGING_ATTEMPTS {
        let nonce = SAVE_NONCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".cinnabar-studio-{}-{nonce}.saving",
            std::process::id()
        ));
        match File::options().create_new(true).write(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(io(&path, source)),
        }
    }
    Err(StorageError::Busy)
}
