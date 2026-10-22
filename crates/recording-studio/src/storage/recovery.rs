//! Ordered autosave publication and explicit recovery retirement never replace a user's saved project.

use super::*;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

mod files;
mod lease;
mod reader;
#[cfg(test)]
mod tests;

pub const AUTOSAVE_INTERVAL_MICROS: u64 = 30 * crate::time::MICROS_PER_SECOND;
pub const MAX_RECOVERY_PROJECTS: usize = 8;
pub const MAX_RECOVERY_VERSIONS: usize = 3;
const RECOVERY_VERSION: u32 = 1;
const MAX_RECOVERY_BYTES: usize = crate::model::MAX_PROJECT_BYTES + 16 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct RecoveryEntry {
    pub id: String,
    pub name: String,
    pub original_file: Option<PathBuf>,
    pub revision: u64,
    /// Highest file revision observed by this listing; later concurrent backups are never discarded implicitly.
    pub discard_through: u64,
    pub modified_unix_ms: u64,
    pub versions: usize,
    pub notice: Option<String>,
    pub recoverable: bool,
}

pub enum RecoveryRequest {
    Begin {
        root: PathBuf,
    },
    Scan {
        root: PathBuf,
    },
    Save {
        root: PathBuf,
        id: String,
        document: u64,
        revision: u64,
        project: Arc<Project>,
        original_file: Option<PathBuf>,
        source_base: PathBuf,
    },
    Saved {
        root: PathBuf,
        document: u64,
        revision: u64,
        known: Option<String>,
        restored: Option<(String, u64)>,
    },
    DiscardDocument {
        root: PathBuf,
        document: u64,
        known: Option<String>,
        restored: Option<(String, u64)>,
    },
    Restore {
        root: PathBuf,
        id: String,
    },
    Discard {
        root: PathBuf,
        id: String,
        revision: u64,
    },
}

pub enum RecoveryValue {
    Opened {
        id: String,
    },
    Entries(Vec<RecoveryEntry>),
    Saved {
        document: u64,
        revision: u64,
    },
    Cleared,
    Checkpoint {
        retained: bool,
    },
    Restored {
        id: String,
        revision: u64,
        editor: Editor,
        notice: String,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    revision: u64,
    modified_unix_ms: u64,
    original_file: Option<PathBuf>,
    project: Project,
}

#[derive(Default)]
pub(super) struct Store {
    active: Option<Active>,
    closed_through: u64,
}
struct Active {
    document: u64,
    slot: Option<PathBuf>,
    saved_revision: Option<u64>,
}

impl Store {
    /// One worker orders writes, saved checkpoints and intentional discards even when the UI changes documents.
    pub fn execute(&mut self, request: RecoveryRequest) -> Result<RecoveryValue, StorageError> {
        let _lease = lease::acquire(request.root())?;
        match request {
            RecoveryRequest::Begin { root } => {
                let slot = files::create_slot(&root)?;
                Ok(RecoveryValue::Opened {
                    id: slot
                        .file_name()
                        .and_then(|name| name.to_str())
                        .expect("generated recovery name")
                        .to_owned(),
                })
            }
            RecoveryRequest::Scan { root } => files::list(&root).map(RecoveryValue::Entries),
            RecoveryRequest::Save {
                root,
                id,
                document,
                revision,
                project,
                original_file,
                source_base,
            } => {
                if document <= self.closed_through
                    || self
                        .active
                        .as_ref()
                        .is_some_and(|active| active.document > document)
                {
                    return Err(StorageError::RecoveryRetired);
                }
                if self
                    .active
                    .as_ref()
                    .is_none_or(|active| active.document != document)
                {
                    self.active = Some(Active {
                        document,
                        slot: Some(files::slot(&root, &id)?),
                        saved_revision: None,
                    });
                }
                let active = self.active.as_mut().expect("active recovery document");
                if active.saved_revision.is_none_or(|saved| revision > saved) {
                    let mut project = (*project).clone();
                    let base = super::files::absolute(&source_base)?;
                    super::paths::relocate(&mut project, &base, None);
                    project.to_json()?;
                    let original_file = original_file
                        .as_deref()
                        .map(super::files::absolute)
                        .transpose()?;
                    let modified_unix_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                        .min(u128::from(u64::MAX))
                        as u64;
                    let snapshot = Snapshot {
                        version: RECOVERY_VERSION,
                        revision,
                        modified_unix_ms,
                        original_file,
                        project,
                    };
                    if active.slot.is_none() {
                        active.slot = Some(files::slot(&root, &id)?);
                    }
                    files::save(active.slot.as_deref().expect("recovery slot"), &snapshot)?;
                }
                Ok(RecoveryValue::Saved { document, revision })
            }
            RecoveryRequest::Saved {
                root,
                document,
                revision,
                known,
                restored,
            } => {
                if let Some(active) = self
                    .active
                    .as_mut()
                    .filter(|active| active.document == document)
                {
                    active.saved_revision = Some(
                        active
                            .saved_revision
                            .map_or(revision, |old| old.max(revision)),
                    );
                    if let Some(slot) = &active.slot {
                        files::remove_through(slot, revision)?;
                    }
                    if active.slot.as_ref().is_some_and(|slot| !slot.exists()) {
                        active.slot = None;
                    }
                }
                let retained = if let Some(id) = known {
                    let slot = files::slot(&root, &id)?;
                    files::remove_through(&slot, revision)?;
                    slot.exists()
                } else {
                    false
                };
                if let Some((id, revision)) = restored {
                    files::remove_through(&files::slot(&root, &id)?, revision)?;
                }
                Ok(RecoveryValue::Checkpoint { retained })
            }
            RecoveryRequest::DiscardDocument {
                root,
                document,
                known,
                restored,
            } => {
                if let Some(active) = self
                    .active
                    .as_ref()
                    .filter(|active| active.document <= document)
                {
                    if let Some(slot) = &active.slot {
                        files::retire(slot)?;
                    }
                }
                if let Some(id) = known {
                    files::retire(&files::slot(&root, &id)?)?;
                }
                if let Some((id, revision)) = restored {
                    files::remove_through(&files::slot(&root, &id)?, revision)?;
                }
                self.closed_through = self.closed_through.max(document);
                if self
                    .active
                    .as_ref()
                    .is_some_and(|active| active.document <= document)
                {
                    self.active = None;
                }
                Ok(RecoveryValue::Cleared)
            }
            RecoveryRequest::Restore { root, id } => {
                let (snapshot, notice, _) = reader::latest(&files::slot(&root, &id)?)?;
                let revision = snapshot.revision;
                let editor = Editor::recovered(snapshot.project)?;
                Ok(RecoveryValue::Restored {
                    id,
                    revision,
                    editor,
                    notice: format!(
                        "Recovered an unsaved copy. Save creates a new project; the original saved file is unchanged. {notice}"
                    ),
                })
            }
            RecoveryRequest::Discard { root, id, revision } => {
                files::remove_through(&files::slot(&root, &id)?, revision)?;
                Ok(RecoveryValue::Cleared)
            }
        }
    }
}
