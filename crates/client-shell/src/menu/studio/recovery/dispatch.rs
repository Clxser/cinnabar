//! Worker request admission captures one immutable revision.

use super::*;

impl Studio {
    /// The request owns one immutable revision; the UI never serializes or clones project contents.
    pub(super) fn dispatch_recovery(&mut self, operation: Operation) -> Result<(), String> {
        let root = self.root.clone();
        let request = match &operation {
            Operation::Scan => RecoveryRequest::Scan { root },
            Operation::Begin(_) => RecoveryRequest::Begin { root },
            Operation::Save(document, revision) => RecoveryRequest::Save {
                root,
                id: self
                    .recovery
                    .slot
                    .clone()
                    .ok_or("Recovery slot is not ready.")?,
                document: *document,
                revision: *revision,
                project: Arc::clone(
                    self.editor
                        .as_ref()
                        .ok_or("No project to autosave.")?
                        .project(),
                ),
                original_file: self.view.file.clone(),
                source_base: self
                    .view
                    .file
                    .as_ref()
                    .and_then(|path| path.parent())
                    .map_or_else(|| self.root.clone(), PathBuf::from),
            },
            Operation::Checkpoint {
                document,
                revision,
                known,
                restored,
            } => RecoveryRequest::Saved {
                root,
                document: *document,
                revision: *revision,
                known: known.clone(),
                restored: restored.clone(),
            },
            Operation::Leave {
                document,
                known,
                restored,
                ..
            } => RecoveryRequest::DiscardDocument {
                root,
                document: *document,
                known: known.clone(),
                restored: restored.clone(),
            },
            Operation::Restore { id, .. } => RecoveryRequest::Restore {
                root,
                id: id.clone(),
            },
            Operation::Discard { id, revision } => RecoveryRequest::Discard {
                root,
                id: id.clone(),
                revision: *revision,
            },
        };
        if self.recovery.service.is_none() {
            self.recovery.service =
                Some(ProjectStorage::start().map_err(|error| error.to_string())?);
        }
        let job = self
            .recovery
            .service
            .as_mut()
            .expect("recovery service")
            .request(StorageRequest::Recovery(request))
            .map_err(|error| error.to_string())?;
        if matches!(operation, Operation::Save(..) | Operation::Begin(_)) {
            self.recovery.clock.submitted();
            Arc::make_mut(&mut self.view).workflow.recovery.autosaving = true;
        }
        self.recovery.pending = Some((job, operation));
        Ok(())
    }
}
