//! A shared filesystem lease orders publication and revision tombstones across app sessions.

use super::*;

const LOCK_FILE: &str = ".storage-lock";
const DISCARDED_FILE: &str = ".discard-through";

impl RecoveryRequest {
    /// All recovery operations share one bounded store and therefore one lease location.
    pub(super) fn root(&self) -> &Path {
        match self {
            Self::Begin { root }
            | Self::Scan { root }
            | Self::Save { root, .. }
            | Self::Saved { root, .. }
            | Self::DiscardDocument { root, .. }
            | Self::Restore { root, .. }
            | Self::Discard { root, .. } => root,
        }
    }
}

/// An occupied store returns immediately; dropping the file releases its operating-system lease.
pub(super) fn acquire(root: &Path) -> Result<fs::File, StorageError> {
    let parent = super::super::files::absolute(root)?.join("recovery");
    files::entries(&parent)?;
    fs::create_dir_all(&parent).map_err(|source| io(&parent, source))?;
    let path = parent.join(LOCK_FILE);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.is_file() || metadata.is_symlink() => {
            return Err(files::error(
                &path,
                "Recovery lease must be a regular file.",
            ));
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(io(&path, error));
        }
        _ => {}
    }
    let file = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|source| io(&path, source))?;
    file.try_lock().map_err(|error| match error {
        fs::TryLockError::WouldBlock => StorageError::Busy,
        fs::TryLockError::Error(source) => io(&path, source),
    })?;
    Ok(file)
}

/// A retained newer snapshot must not allow an older writer to republish a discarded revision.
pub(super) fn discarded_through(slot: &Path) -> Result<Option<u64>, StorageError> {
    use std::io::Read;
    let path = slot.join(DISCARDED_FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(io(&path, source)),
    };
    if !metadata.is_file() || metadata.is_symlink() || metadata.len() > 20 {
        return Err(files::error(&path, "Invalid recovery discard checkpoint."));
    }
    let mut value = String::new();
    fs::File::open(&path)
        .map_err(|source| io(&path, source))?
        .take(21)
        .read_to_string(&mut value)
        .map_err(|source| io(&path, source))?;
    value
        .parse()
        .map(Some)
        .map_err(|_| files::error(&path, "Invalid recovery discard checkpoint."))
}

/// The durable threshold precedes file deletion while the caller holds the shared store lease.
pub(super) fn discard_through(slot: &Path, revision: u64) -> Result<(), StorageError> {
    let revision = discarded_through(slot)?.map_or(revision, |old| old.max(revision));
    super::super::files::atomic_existing(
        &slot.join(DISCARDED_FILE),
        revision.to_string().as_bytes(),
    )
}

/// Only the generated tombstone is eligible for retired-slot cleanup.
pub(super) fn is_checkpoint(name: &str) -> bool {
    name == DISCARDED_FILE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_durable_discard_checkpoint_cannot_expose_an_old_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::default();
        let RecoveryValue::Opened { id } = store
            .execute(RecoveryRequest::Begin {
                root: root.path().into(),
            })
            .unwrap()
        else {
            panic!("opened slot");
        };
        store
            .execute(RecoveryRequest::Save {
                root: root.path().into(),
                id: id.clone(),
                document: 1,
                revision: 1,
                project: Arc::new(Project::new("must remain hidden")),
                original_file: None,
                source_base: root.path().into(),
            })
            .unwrap();
        let slot = files::slot(root.path(), &id).unwrap();
        fs::write(slot.join(DISCARDED_FILE), b"invalid").unwrap();
        assert!(reader::latest(&slot).is_err());
        assert!(!files::list(root.path()).unwrap()[0].recoverable);
    }
}
