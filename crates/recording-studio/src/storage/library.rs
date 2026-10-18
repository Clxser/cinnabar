//! Bounded recent-project enumeration. Media and world checkpoints are opened only when selected.

use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use super::{StorageError, io};
use crate::{
    archive::{RECORDING_EXTENSION, RecordingInfo},
    model::PROJECT_EXTENSION,
};

mod query;
pub use query::{LibraryQuery, LibrarySort, MAX_LIBRARY_SEARCH_BYTES};

const MAX_LIBRARY_ENTRIES: usize = 256;
const MAX_DIRECTORY_ENTRIES: usize = 4096;
const MAX_LIBRARY_ISSUES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LibraryKind {
    Project,
    Recording,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LibraryEntry {
    pub path: PathBuf,
    pub name: String,
    pub kind: LibraryKind,
    pub modified_unix_ms: u64,
    pub interrupted: bool,
}

#[derive(Clone, Debug, Default)]
pub struct LibraryListing {
    pub entries: Vec<LibraryEntry>,
    pub issues: Vec<String>,
    /// The library must show that its listing is incomplete, with an option to open a file directly.
    pub truncated: bool,
}

/// Keeps enumeration bounded; one malformed recording does not hide other projects.
pub(super) fn list(root: &Path, query: &LibraryQuery) -> Result<LibraryListing, StorageError> {
    query.validate()?;
    let mut listing = LibraryListing::default();
    let directory = match fs::read_dir(root) {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(listing),
        Err(source) => return Err(io(root, source)),
    };
    for (index, entry) in directory.enumerate() {
        if index >= MAX_DIRECTORY_ENTRIES {
            listing.truncated = true;
            break;
        }
        let result = entry
            .map_err(|source| io(root, source))
            .and_then(|entry| inspect(&entry.path()));
        match result {
            Ok(Some(entry)) => listing.entries.push(entry),
            Ok(None) => {}
            Err(error) => {
                if listing.issues.len() < MAX_LIBRARY_ISSUES {
                    listing.issues.push(error.to_string());
                } else {
                    listing.truncated = true;
                }
            }
        }
    }
    query::finish(&mut listing, query);
    Ok(listing)
}

/// Reads only the title of a recording, not its potentially large world index or segment payloads.
fn inspect(path: &Path) -> Result<Option<LibraryEntry>, StorageError> {
    let kind = match path.extension().and_then(|extension| extension.to_str()) {
        Some(PROJECT_EXTENSION) => LibraryKind::Project,
        Some(RECORDING_EXTENSION) => LibraryKind::Recording,
        _ => return Ok(None),
    };
    let metadata = fs::symlink_metadata(path).map_err(|source| io(path, source))?;
    if kind == LibraryKind::Project && !metadata.is_file()
        || kind == LibraryKind::Recording && !metadata.is_dir()
    {
        return Ok(None);
    }
    let mut interrupted = false;
    let name = if kind == LibraryKind::Recording {
        let info = RecordingInfo::read(path)?;
        interrupted = info.scene_schema == crate::capture::POV_SCENE_SCHEMA
            && !crate::capture::CaptureManifest::exists(path);
        info.name
    } else {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("Untitled project")
            .to_owned()
    };
    let modified_unix_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| {
            duration.as_millis().min(u128::from(u64::MAX)) as u64
        });
    Ok(Some(LibraryEntry {
        path: path.to_path_buf(),
        name,
        kind,
        modified_unix_ms,
        interrupted,
    }))
}

/// Builds only the newest bounded sheet; unavailable thumbnails keep the entry's textual identity.
pub(super) fn thumbnails(
    entries: &[LibraryEntry],
    cancellation: &crate::export::Cancellation,
) -> Result<super::StorageValue, StorageError> {
    use crate::media::{MAX_THUMBNAILS, THUMBNAIL_COLUMNS, THUMBNAIL_SIZE, ThumbnailSheet};
    let [width, height] = THUMBNAIL_SIZE;
    let rows = entries
        .len()
        .min(MAX_THUMBNAILS)
        .div_ceil(THUMBNAIL_COLUMNS)
        .max(1);
    let size = [width * THUMBNAIL_COLUMNS as u32, height * rows as u32];
    let mut rgba = vec![0_u8; (size[0] * size[1] * 4) as usize];
    let mut slots = vec![None; entries.len()];
    for (index, entry) in entries.iter().take(MAX_THUMBNAILS).enumerate() {
        if cancellation.cancelled() {
            break;
        }
        let source = match entry.kind {
            LibraryKind::Recording => Some(entry.path.join(crate::capture::VIDEO_FILE)),
            LibraryKind::Project => super::files::load(&entry.path)
                .ok()
                .and_then(|project| {
                    project.sources.into_iter().find(|source| {
                        matches!(
                            source.kind,
                            crate::model::MediaKind::Video | crate::model::MediaKind::Image
                        )
                    })
                })
                .map(|source| {
                    if source.path.is_absolute() {
                        source.path
                    } else {
                        entry
                            .path
                            .parent()
                            .unwrap_or(Path::new("."))
                            .join(source.path)
                    }
                }),
        };
        let Some(pixels) =
            source.and_then(|path| crate::media::thumbnail(&path, cancellation).ok())
        else {
            continue;
        };
        let x = (index % THUMBNAIL_COLUMNS) as u32 * width;
        let y = (index / THUMBNAIL_COLUMNS) as u32 * height;
        for row in 0..height {
            let from = (row * width * 4) as usize;
            let to = ((y + row) * size[0] * 4 + x * 4) as usize;
            rgba[to..to + (width * 4) as usize]
                .copy_from_slice(&pixels[from..from + (width * 4) as usize]);
        }
        slots[index] = Some([x as u16, y as u16, (x + width) as u16, (y + height) as u16]);
    }
    Ok(super::StorageValue::Thumbnails {
        sheet: std::sync::Arc::new(ThumbnailSheet {
            size,
            rgba8: rgba.into(),
        }),
        slots,
    })
}
