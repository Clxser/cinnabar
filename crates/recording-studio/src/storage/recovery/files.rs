//! Every recovery path is a validated direct child; retirement first renames the whole slot out of discovery.

use super::*;

/// Diagnostic context stays attached to the affected recovery path.
pub(super) fn error(path: &Path, message: &str) -> StorageError {
    io(path, std::io::Error::other(message))
}

/// Recovery identifiers cannot address a saved project, another directory or a parent path.
pub(super) fn slot(root: &Path, id: &str) -> Result<PathBuf, StorageError> {
    if !id.starts_with("project-")
        || id.len() > 80
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(error(
            root,
            "Invalid recovery identifier. Refresh the recovery list.",
        ));
    }
    let parent = super::super::files::absolute(root)?.join("recovery");
    entries(&parent)?;
    Ok(parent.join(id))
}

/// Enumeration is bounded before collecting or inspecting metadata; symlinked recovery folders are rejected.
pub(super) fn entries(path: &Path) -> Result<Vec<fs::DirEntry>, StorageError> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_symlink() || !meta.is_dir() => {
            return Err(error(
                path,
                "Recovery storage must be a real directory, not a link.",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(io(path, source)),
        _ => {}
    }
    let mut result = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|source| io(path, source))?
        .take(MAX_DIRECTORY_ENTRIES + 1)
    {
        result.push(entry.map_err(|source| io(path, source))?);
    }
    if result.len() > MAX_DIRECTORY_ENTRIES {
        return Err(error(
            path,
            "Recovery directory exceeds its scan limit. Preserve its files and remove unrelated entries before retrying.",
        ));
    }
    Ok(result)
}

/// Empty and interrupted retirement directories are cleaned without following links or deleting unknown files.
pub(super) fn create_slot(root: &Path) -> Result<PathBuf, StorageError> {
    let parent = super::super::files::absolute(root)?.join("recovery");
    fs::create_dir_all(&parent).map_err(|source| io(&parent, source))?;
    let count = entries(&parent)?
        .into_iter()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("project-"))
        })
        .count();
    if count >= MAX_RECOVERY_PROJECTS {
        return Err(error(
            &parent,
            &format!(
                "Recovery storage holds its {MAX_RECOVERY_PROJECTS}-project limit. Restore or discard an older recovery before retrying autosave."
            ),
        ));
    }
    let directory = tempfile::Builder::new()
        .prefix("project-")
        .tempdir_in(&parent)
        .map_err(|source| io(&parent, source))?;
    let slot = directory.keep();
    super::super::files::sync_parent(&parent)?;
    Ok(slot)
}

/// Known slots are never recreated after retirement, so another app cannot resurrect an intentional discard.
pub(super) fn save(path: &Path, snapshot: &Snapshot) -> Result<(), StorageError> {
    entries(path)?;
    if !path.is_dir() {
        return Err(StorageError::RecoveryRetired);
    }
    if lease::discarded_through(path)?.is_some_and(|revision| snapshot.revision <= revision) {
        return Err(StorageError::RecoveryRetired);
    }
    let bytes = serde_json::to_vec(snapshot).map_err(StudioError::from)?;
    if bytes.len() > MAX_RECOVERY_BYTES {
        return Err(error(path, "Recovery metadata exceeds its byte budget."));
    }
    let file = path.join(format!("revision-{:020}.json", snapshot.revision));
    super::super::files::atomic_existing(&file, &bytes)?;
    let mut versions = versions(path)?;
    versions.sort_by_key(|(revision, _)| *revision);
    let remove = versions.len().saturating_sub(MAX_RECOVERY_VERSIONS);
    for (_, file) in versions.into_iter().take(remove) {
        fs::remove_file(&file).map_err(|source| io(&file, source))?;
    }
    super::super::files::sync_parent(path)
}

/// File names are parsed instead of trusting their lexical order or following arbitrary directory entries.
pub(super) fn versions(path: &Path) -> Result<Vec<(u64, PathBuf)>, StorageError> {
    let mut found = Vec::new();
    for entry in entries(path)? {
        let name = entry.file_name();
        let Some(revision) = name
            .to_str()
            .and_then(|name| name.strip_prefix("revision-"))
            .and_then(|name| name.strip_suffix(".json"))
            .and_then(|value| value.parse::<u64>().ok())
        else {
            continue;
        };
        let metadata = entry
            .file_type()
            .map_err(|source| io(&entry.path(), source))?;
        if !metadata.is_file() {
            return Err(error(
                &entry.path(),
                "Recovery snapshots must be regular files.",
            ));
        }
        found.push((revision, entry.path()));
    }
    found.sort_by_key(|(revision, _)| std::cmp::Reverse(*revision));
    Ok(found)
}

/// A save removes only snapshots it durably superseded; newer edited revisions remain recoverable.
pub(super) fn remove_through(path: &Path, revision: u64) -> Result<(), StorageError> {
    if !path.exists() {
        return Ok(());
    }
    entries(path)?;
    lease::discard_through(path, revision)?;
    for (stored, file) in versions(path)? {
        if stored <= revision {
            fs::remove_file(&file).map_err(|source| io(&file, source))?;
        }
    }
    if versions(path)?.is_empty() {
        retire(path)?;
    }
    Ok(())
}

/// Rename is the persistent discard acknowledgement: the old slot path can never publish a discoverable snapshot again.
pub(super) fn retire(path: &Path) -> Result<(), StorageError> {
    if !path.exists() {
        return path
            .parent()
            .map_or(Ok(()), super::super::files::sync_parent);
    }
    entries(path)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(StorageError::InvalidPath)?;
    let retired = path.with_file_name(format!(".retired-{name}"));
    fs::rename(path, &retired).map_err(|source| io(path, source))?;
    super::super::files::sync_parent(path.parent().ok_or(StorageError::InvalidPath)?)?;
    // Only files with this owner's generated names are eligible for cleanup.
    cleanup(&retired);
    Ok(())
}

/// Cleanup failure keeps a hidden retired slot; it cannot reappear in the recovery list.
fn cleanup(path: &Path) {
    if let Ok(entries) = entries(path) {
        for entry in entries {
            let name = entry.file_name();
            if entry.file_type().is_ok_and(|kind| kind.is_file())
                && name.to_str().is_some_and(|name| {
                    name.starts_with("revision-") && name.ends_with(".json")
                        || name.starts_with(".cinnabar-studio-") && name.ends_with(".saving")
                        || lease::is_checkpoint(name)
                })
            {
                let _ = fs::remove_file(entry.path());
            }
        }
        let _ = fs::remove_dir(path);
    }
}

/// Recovery discovery leaves malformed folders intact and makes their errors actionable rather than silently hiding them.
pub(super) fn list(root: &Path) -> Result<Vec<RecoveryEntry>, StorageError> {
    let parent = super::super::files::absolute(root)?.join("recovery");
    let mut found = Vec::new();
    for entry in entries(&parent)? {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(".retired-project-") {
            cleanup(&entry.path());
            continue;
        }
        if !name.starts_with("project-") {
            continue;
        }
        match slot(root, name).and_then(|slot| reader::latest(&slot)) {
            Ok((snapshot, notice, versions)) => found.push(RecoveryEntry {
                id: name.to_owned(),
                name: snapshot.project.name,
                original_file: snapshot.original_file,
                revision: snapshot.revision,
                discard_through: self::versions(&entry.path())?
                    .first()
                    .map_or(snapshot.revision, |(revision, _)| *revision),
                modified_unix_ms: snapshot.modified_unix_ms,
                versions,
                notice: (!notice.is_empty()).then_some(notice),
                recoverable: true,
            }),
            Err(failure) => {
                let stored = versions(&entry.path()).unwrap_or_default();
                found.push(RecoveryEntry {
                    id: name.to_owned(),
                    name: "Unreadable recovery".into(),
                    original_file: None,
                    revision: 0,
                    discard_through: stored.first().map_or(0, |(revision, _)| *revision),
                    modified_unix_ms: 0,
                    versions: stored.len(),
                    notice: Some(failure.to_string()),
                    recoverable: false,
                });
            }
        }
        if found.len() > MAX_RECOVERY_PROJECTS {
            return Err(error(
                &parent,
                "Too many recovery projects. Preserve this folder and remove unrelated entries before retrying.",
            ));
        }
    }
    found.sort_by_key(|entry| std::cmp::Reverse(entry.modified_unix_ms));
    Ok(found)
}
