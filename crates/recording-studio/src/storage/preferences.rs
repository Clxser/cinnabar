//! Small user capture preferences share the storage worker and atomic-file publication policy.

use super::{StorageError, io};
use crate::capture::CaptureSettings;
use std::{
    io::{Read, Write},
    path::Path,
};
const FILE: &str = "capture-settings.json";
const MAX_BYTES: u64 = 4096;

/// Missing preferences retain the established recording defaults; malformed preferences are visible.
pub(super) fn load(root: &Path) -> Result<CaptureSettings, StorageError> {
    let path = root.join(FILE);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CaptureSettings::default());
        }
        Err(error) => return Err(io(&path, error)),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io(&path, error))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(io(
            &path,
            std::io::Error::other("Capture preferences exceed their byte limit."),
        ));
    }
    let settings: CaptureSettings =
        serde_json::from_slice(&bytes).map_err(|error| io(&path, std::io::Error::other(error)))?;
    settings
        .validate()
        .map_err(|error| io(&path, std::io::Error::other(error)))?;
    Ok(settings)
}

/// Synced staging preserves the last complete preferences if an interrupted write cannot publish.
pub(super) fn save(root: &Path, settings: CaptureSettings) -> Result<(), StorageError> {
    let path = root.join(FILE);
    settings
        .validate()
        .map_err(|error| io(&path, std::io::Error::other(error)))?;
    std::fs::create_dir_all(root).map_err(|error| io(root, error))?;
    let bytes =
        serde_json::to_vec(&settings).map_err(|error| io(&path, std::io::Error::other(error)))?;
    let mut staged = tempfile::NamedTempFile::new_in(root).map_err(|error| io(root, error))?;
    staged
        .write_all(&bytes)
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|error| io(&path, error))?;
    staged
        .persist(&path)
        .map_err(|error| io(&path, error.error))?;
    Ok(())
}
