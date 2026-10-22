//! Explicit save publication retains its immutable snapshot until the worker acknowledges it.

use super::*;

#[derive(Clone)]
pub(super) struct SaveJob {
    pub job: u64,
    pub document: u64,
    pub project: Arc<Project>,
}

impl Studio {
    /// Starts the service lazily so ordinary gameplay and launcher startup incur no studio IO.
    pub(super) fn ensure_storage(&mut self) -> Result<(), String> {
        if self.storage.is_none() {
            self.storage = Some(ProjectStorage::start().map_err(|e| e.to_string())?);
        }
        Ok(())
    }

    /// Coalesces repeated refresh presses into one outstanding library job.
    pub(super) fn refresh(&mut self) -> Result<(), String> {
        if self.library_job.is_some() {
            return Ok(());
        }
        let generation = self
            .view
            .workspace
            .library
            .generation
            .checked_add(1)
            .ok_or("Library generation is exhausted. Restart Cinnabar.")?;
        self.ensure_storage()?;
        self.thumbnails_cancel.cancel();
        self.thumbnails_job = None;
        self.library_job = Some(
            self.storage
                .as_mut()
                .expect("storage exists")
                .request(StorageRequest::ListLibrary {
                    root: self.root.clone(),
                    query: self.view.workspace.library.query(),
                })
                .map_err(|e| e.to_string())?,
        );
        let view = Arc::make_mut(&mut self.view);
        view.workspace.library.generation = generation;
        view.library_loading = true;
        view.library = Arc::default();
        view.library_truncated = false;
        view.workflow.thumbnail_sheet = None;
        view.workflow.thumbnail_slots = Arc::default();
        Ok(())
    }

    /// Captures an immutable revision; opening another document cannot misapply its save completion.
    pub(super) fn save(&mut self) -> Result<(), String> {
        if self.save_job.is_some() {
            return Ok(());
        }
        self.ensure_storage()?;
        self.thumbnails_cancel.cancel();
        let editor = self
            .editor
            .as_ref()
            .ok_or("Create or open a project first.")?;
        let project = Arc::clone(editor.project());
        let revision = editor.revision();
        let request = if let Some(path) = &self.view.file {
            StorageRequest::SaveProject {
                path: path.clone(),
                project,
                revision,
                source_base: self
                    .view
                    .file
                    .as_ref()
                    .and_then(|path| path.parent())
                    .map_or_else(|| self.root.clone(), PathBuf::from),
            }
        } else {
            StorageRequest::SaveNewProject {
                directory: self.root.clone(),
                project,
                revision,
                source_base: self.root.clone(),
            }
        };
        let job = self
            .storage
            .as_mut()
            .expect("storage exists")
            .request(request)
            .map_err(|e| e.to_string())?;
        self.save_job = Some(SaveJob {
            job,
            document: self.document,
            project: Arc::clone(self.editor.as_ref().expect("saved editor").project()),
        });
        Arc::make_mut(&mut self.view).saving = true;
        Ok(())
    }

    /// Publishes only the newest matching job; a failed open leaves the existing editor intact.
    pub(super) fn completed(&mut self, completed: StorageResult) {
        let library = self.library_job == Some(completed.job);
        let open = self.open_job == Some(completed.job);
        let saved = self
            .save_job
            .as_ref()
            .filter(|saved| saved.job == completed.job)
            .cloned();
        let preferences = self.preferences_job == Some(completed.job);
        let thumbnails = self.thumbnails_job == Some(completed.job);
        if !library && !open && saved.is_none() && !preferences && !thumbnails {
            return;
        }
        if preferences {
            self.preferences_job = None;
        }
        if thumbnails {
            self.thumbnails_job = None;
        }
        if library {
            self.library_job = None;
            Arc::make_mut(&mut self.view).library_loading = false;
        }
        if open {
            self.open_job = None;
            Arc::make_mut(&mut self.view).loading = false;
        }
        if saved.is_some() {
            self.save_job = None;
            Arc::make_mut(&mut self.view).saving = false;
        }
        let value = match completed.result {
            Ok(value) => value,
            Err(error) => {
                bevy::log::warn!(error = %error, "Studio storage operation failed");
                Arc::make_mut(&mut self.view).error = Some(error.to_string());
                self.warn_retained_recovery();
                return;
            }
        };
        match value {
            StorageValue::Library(listing) => {
                let view = Arc::make_mut(&mut self.view);
                view.library = listing.entries.into();
                view.library_truncated = listing.truncated;
                view.notice = (!listing.issues.is_empty()).then(|| listing.issues.join("\n"));
                view.workflow.thumbnail_sheet = None;
                view.workflow.thumbnail_slots = Arc::default();
                self.thumbnails_cancel = Default::default();
                self.thumbnails_job = self.storage.as_mut().and_then(|storage| {
                    storage
                        .request(StorageRequest::Thumbnails {
                            entries: self.view.library.clone(),
                            cancellation: self.thumbnails_cancel.clone(),
                        })
                        .ok()
                });
            }
            StorageValue::Thumbnails { sheet, slots } => {
                let workflow = &mut Arc::make_mut(&mut self.view).workflow;
                workflow.thumbnail_sheet = Some(sheet);
                workflow.thumbnail_slots = slots.into();
            }
            StorageValue::CaptureSettings(settings) => {
                if !self.preferences_changed {
                    Arc::make_mut(&mut self.view).workflow.capture_settings = settings;
                }
            }
            StorageValue::LoadedProject { path, editor } => self.install(editor, Some(path)),
            StorageValue::CapturedVideo { editor, notice } => {
                self.install(editor, None);
                Arc::make_mut(&mut self.view).notice = notice;
            }
            StorageValue::SavedProject { path, revision } => {
                if let Some(saved) = saved.filter(|saved| saved.document == self.document) {
                    if let Some(editor) = &mut self.editor {
                        editor.mark_saved_snapshot(saved.project);
                    }
                    self.recovery_saved(revision);
                    Arc::make_mut(&mut self.view).file = Some(path);
                    let _ = self.refresh();
                }
            }
            StorageValue::Recording { path, reader } => {
                if let Err(error) = self.import_replay(path, reader) {
                    Arc::make_mut(&mut self.view).error = Some(error);
                }
            }
            StorageValue::Recovery(_) => {}
        }
        self.publish_editor();
    }
}
