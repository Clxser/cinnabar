//! Export files use exclusive staging and no-clobber publication beside the chosen destination.

use super::*;

/// Resolve relative destinations once on the IO worker, before any child process is started.
pub(super) fn parent(destination: &Destination) -> Result<PathBuf, String> {
    let parent = match destination {
        Destination::Directory(path) => path.as_path(),
        Destination::File(path) => {
            if path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_none_or(|extension| !extension.eq_ignore_ascii_case("mp4"))
            {
                return Err("Choose an MP4 output filename.".into());
            }
            path.parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
        }
    };
    if parent.is_absolute() {
        Ok(parent.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|base| base.join(parent))
            .map_err(|error| error.to_string())
    }
}

/// Closing the reservation handle allows FFmpeg to open this file on Windows.
pub(super) fn staging(parent: &Path) -> Result<tempfile::TempPath, String> {
    tempfile::Builder::new()
        .prefix(".cinnabar-export-")
        .suffix(".partial.mp4")
        .tempfile_in(parent)
        .map(tempfile::NamedTempFile::into_temp_path)
        .map_err(|error| error.to_string())
}

/// The export worker periodically observes its own file, and checks it again after encoder flush.
pub(super) fn check_bytes(path: &Path, maximum: u64) -> Result<(), String> {
    if fs::metadata(path).map_err(|error| error.to_string())?.len() > maximum {
        Err("Export exceeded its output storage limit.".into())
    } else {
        Ok(())
    }
}

/// Existing files are preserved even when multiple Cinnabar sessions export simultaneously.
pub(super) fn publish(
    mut staged: tempfile::TempPath,
    destination: &Destination,
) -> Result<PathBuf, String> {
    let parent = parent(destination)?;
    for index in 1..=16_384 {
        let path = match destination {
            Destination::File(path) => {
                if path.is_absolute() {
                    path.clone()
                } else {
                    std::env::current_dir()
                        .map_err(|error| error.to_string())?
                        .join(path)
                }
            }
            Destination::Directory(_) => parent.join(format!("Export {index}.mp4")),
        };
        match staged.persist_noclobber(&path) {
            Ok(()) => {
                #[cfg(unix)]
                File::open(&parent)
                    .and_then(|file| file.sync_all())
                    .map_err(|error| {
                        format!(
                            "Video was written to {} but final folder sync failed: {error}",
                            path.display()
                        )
                    })?;
                return Ok(path);
            }
            Err(error)
                if error.error.kind() == std::io::ErrorKind::AlreadyExists
                    && matches!(destination, Destination::Directory(_)) =>
            {
                staged = error.path
            }
            Err(error) => return Err(format!("Cannot publish exported video: {}", error.error)),
        }
    }
    Err("The export directory has no available generated filenames.".into())
}
