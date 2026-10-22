//! Asynchronous project persistence and library enumeration; UI callers never wait on filesystem IO.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, TryLockError,
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
};

use crate::{
    Project, StudioError,
    archive::{ArchiveError, RecordingReader},
    edit::Editor,
};

mod files;
mod library;
mod preferences;
mod recovery;
pub use recovery::{AUTOSAVE_INTERVAL_MICROS, RecoveryEntry, RecoveryRequest, RecoveryValue};

pub use library::{
    LibraryEntry, LibraryKind, LibraryListing, LibraryQuery, LibrarySort, MAX_LIBRARY_SEARCH_BYTES,
};

const STORAGE_QUEUE_CAPACITY: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("project storage is busy; retry when the current operation finishes")]
    Busy,
    #[error("project storage worker is unavailable")]
    Closed,
    #[error(
        "this recovery slot was retired; retry autosave explicitly to protect the open editor in a new slot"
    )]
    RecoveryRetired,
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Project(#[from] StudioError),
    #[error("{path}: {source}")]
    InvalidProject {
        path: PathBuf,
        #[source]
        source: StudioError,
    },
    #[error(transparent)]
    Recording(#[from] ArchiveError),
    #[error("project path has no parent directory")]
    InvalidPath,
    #[error("generated project names are exhausted; choose another folder")]
    NamesExhausted,
}

pub enum StorageRequest {
    Recovery(RecoveryRequest),
    SaveNewProject {
        directory: PathBuf,
        project: Arc<Project>,
        revision: u64,
        source_base: PathBuf,
    },
    LoadProject {
        path: PathBuf,
    },
    SaveProject {
        path: PathBuf,
        project: Arc<Project>,
        revision: u64,
        /// Resolves any relative media paths in the in-memory snapshot; Save As keeps their meaning.
        source_base: PathBuf,
    },
    ListLibrary {
        root: PathBuf,
        query: LibraryQuery,
    },
    OpenRecording {
        path: PathBuf,
    },
    RecoverRecording {
        path: PathBuf,
        cancellation: crate::export::Cancellation,
    },
    LoadCaptureSettings {
        root: PathBuf,
    },
    SaveCaptureSettings {
        root: PathBuf,
        settings: crate::capture::CaptureSettings,
    },
    Thumbnails {
        entries: Arc<[LibraryEntry]>,
        cancellation: crate::export::Cancellation,
    },
}

pub enum StorageValue {
    Recovery(RecoveryValue),
    LoadedProject {
        path: PathBuf,
        editor: Editor,
    },
    SavedProject {
        path: PathBuf,
        revision: u64,
    },
    Library(LibraryListing),
    CaptureSettings(crate::capture::CaptureSettings),
    Thumbnails {
        sheet: Arc<crate::media::ThumbnailSheet>,
        slots: Vec<Option<[u16; 4]>>,
    },
    Recording {
        path: PathBuf,
        reader: Arc<RecordingReader>,
    },
    CapturedVideo {
        editor: Editor,
        notice: Option<String>,
    },
}

pub struct StorageResult {
    pub job: u64,
    pub result: Result<StorageValue, StorageError>,
}

/// One worker serializes file publications. Job IDs let a UI reject stale loads and saves safely.
pub struct ProjectStorage {
    requests: SyncSender<(u64, StorageRequest)>,
    results: Mutex<Receiver<StorageResult>>,
    next_job: u64,
}

impl ProjectStorage {
    /// Starts no recording and opens no files; all work begins through a bounded request.
    pub fn start() -> Result<Self, StorageError> {
        let (requests, jobs) = mpsc::sync_channel(STORAGE_QUEUE_CAPACITY);
        let (completed, results) = mpsc::sync_channel(STORAGE_QUEUE_CAPACITY);
        std::thread::Builder::new()
            .name("studio-project-storage".into())
            .spawn(move || {
                let mut recovery = recovery::Store::default();
                while let Ok((job, request)) = jobs.recv() {
                    let result = match request {
                        StorageRequest::Recovery(request) => {
                            recovery.execute(request).map(StorageValue::Recovery)
                        }
                        request => execute(request),
                    };
                    if completed.send(StorageResult { job, result }).is_err() {
                        break;
                    }
                }
            })
            .map_err(|source| StorageError::Io {
                path: PathBuf::from("studio-project-storage"),
                source,
            })?;
        Ok(Self {
            requests,
            results: Mutex::new(results),
            next_job: 1,
        })
    }

    /// Does not block on the worker or clone media. A failed enqueue has no filesystem side effects.
    pub fn request(&mut self, request: StorageRequest) -> Result<u64, StorageError> {
        let job = self.next_job;
        let next = job.checked_add(1).ok_or(StorageError::Closed)?;
        self.requests
            .try_send((job, request))
            .map_err(|error| match error {
                TrySendError::Full(_) => StorageError::Busy,
                TrySendError::Disconnected(_) => StorageError::Closed,
            })?;
        self.next_job = next;
        Ok(job)
    }

    /// Returns at most one completion so the caller can choose its own per-frame service budget.
    pub fn poll(&self) -> Result<Option<StorageResult>, StorageError> {
        let results = match self.results.try_lock() {
            Ok(results) => results,
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
        };
        match results.try_recv() {
            Ok(result) => Ok(Some(result)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(StorageError::Closed),
        }
    }
}

/// Runs only on the storage worker; callers consume structured completions on their own schedule.
fn execute(request: StorageRequest) -> Result<StorageValue, StorageError> {
    match request {
        StorageRequest::Recovery(_) => unreachable!("recovery owns its ordered worker state"),
        StorageRequest::LoadProject { path } => {
            let path = files::absolute(&path)?;
            let mut project = files::load(&path)?;
            let parent = path.parent().ok_or(StorageError::InvalidPath)?;
            for source in &mut project.sources {
                if source.path.is_relative() {
                    source.path = parent.join(&source.path);
                }
            }
            project.validate()?;
            let editor = Editor::new(project)?;
            Ok(StorageValue::LoadedProject { path, editor })
        }
        StorageRequest::SaveNewProject {
            directory,
            project,
            revision,
            source_base,
        } => {
            let path = files::save_new(
                &files::absolute(&directory)?,
                &project,
                &files::absolute(&source_base)?,
            )?;
            Ok(StorageValue::SavedProject { path, revision })
        }
        StorageRequest::SaveProject {
            path,
            project,
            revision,
            source_base,
        } => {
            let path = files::absolute(&path)?;
            files::save(&path, &project, &files::absolute(&source_base)?)?;
            Ok(StorageValue::SavedProject { path, revision })
        }
        StorageRequest::ListLibrary { root, query } => {
            Ok(StorageValue::Library(library::list(&root, &query)?))
        }
        StorageRequest::Thumbnails {
            entries,
            cancellation,
        } => library::thumbnails(&entries, &cancellation),
        StorageRequest::LoadCaptureSettings { root } => {
            preferences::load(&root).map(StorageValue::CaptureSettings)
        }
        StorageRequest::SaveCaptureSettings { root, settings } => {
            preferences::save(&root, settings)?;
            Ok(StorageValue::CaptureSettings(settings))
        }
        StorageRequest::RecoverRecording { path, cancellation } => {
            let path = files::absolute(&path)?;
            let info = crate::archive::RecordingInfo::read(&path)?;
            if info.scene_schema != crate::capture::POV_SCENE_SCHEMA {
                return Err(io(
                    &path,
                    std::io::Error::other("This recording has no recoverable POV track."),
                ));
            }
            let editor = crate::capture::recover_project(&path, &info.name, &cancellation)
                .map_err(|error| io(&path, std::io::Error::other(error)))?;
            Ok(StorageValue::CapturedVideo { editor, notice: Some("Recovered a verified video prefix into a separate file. Scene tracks, unfinished audio, event markers and missing fragments were not recovered. Save this project to keep the edit.".into()) })
        }
        StorageRequest::OpenRecording { path } => {
            let path = files::absolute(&path)?;
            let reader = Arc::new(RecordingReader::open(&path)?);
            if reader.info().scene_schema == crate::capture::POV_SCENE_SCHEMA {
                let manifest = crate::capture::CaptureManifest::read(&path)
                    .map_err(|error| io(&path, std::io::Error::other(error)))?;
                let (editor, notice) = manifest.project(&reader.info().name, &path)?;
                return Ok(StorageValue::CapturedVideo { editor, notice });
            }
            Ok(StorageValue::Recording { path, reader })
        }
    }
}

/// Attaches the path to every project IO error without printing unrelated environment data.
fn io(path: &Path, source: std::io::Error) -> StorageError {
    StorageError::Io {
        path: path.to_path_buf(),
        source,
    }
}
