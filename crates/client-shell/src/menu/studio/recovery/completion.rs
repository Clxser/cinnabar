//! Job and document identities guard every asynchronous recovery completion.

use super::*;

impl Studio {
    /// Every completion is document guarded before it can install metadata, clear dirty state or leave an editor.
    pub(super) fn recovery_completed(&mut self, result: StorageResult) {
        if self
            .recovery
            .pending
            .as_ref()
            .is_none_or(|(job, _)| *job != result.job)
        {
            return;
        }
        let (_, operation) = self.recovery.pending.take().expect("matched recovery job");
        let is_restore = matches!(operation, Operation::Restore { .. });
        {
            let view = &mut Arc::make_mut(&mut self.view).workflow.recovery;
            if matches!(operation, Operation::Scan) {
                view.loading = self.recovery.scan;
            }
            view.busy = false;
            view.autosaving = false;
        }
        let value = match result.result {
            Ok(StorageValue::Recovery(value)) => value,
            Ok(_) => {
                Arc::make_mut(&mut self.view).workflow.recovery.error =
                    Some("Recovery returned an unexpected result. The editor is unchanged.".into());
                if is_restore {
                    Arc::make_mut(&mut self.view).loading = false;
                }
                self.warn_retained_recovery();
                return;
            }
            Err(error) => {
                if matches!(operation, Operation::Checkpoint { .. }) {
                    self.recovery.failed_checkpoint = Some(operation.clone());
                }
                if matches!(
                    error,
                    recording_studio::storage::StorageError::RecoveryRetired
                ) {
                    self.recovery.retired = true;
                }
                let message = format!("{error}. Your open editor and saved project are unchanged.");
                self.record_recovery_warning(&operation, &message);
                Arc::make_mut(&mut self.view).workflow.recovery.error = Some(message);
                if is_restore {
                    Arc::make_mut(&mut self.view).loading = false;
                }
                self.warn_retained_recovery();
                return;
            }
        };
        self.resolve_recovery_warning(&operation);
        match (operation, value) {
            (Operation::Scan, RecoveryValue::Entries(entries)) => {
                let entries = entries
                    .into_iter()
                    .filter(|entry| {
                        self.recovery.slot.as_deref() != Some(entry.id.as_str())
                            && !self
                                .recovery
                                .restored
                                .as_ref()
                                .is_some_and(|(id, revision)| {
                                    *id == entry.id && entry.revision <= *revision
                                })
                    })
                    .collect::<Vec<_>>();
                Arc::make_mut(&mut self.view).workflow.recovery.entries = entries.into();
            }
            (Operation::Begin(document), RecoveryValue::Opened { id })
                if document == self.document =>
            {
                self.recovery.slot = Some(id);
                self.recovery.clock.retry();
            }
            (
                Operation::Save(document, revision),
                RecoveryValue::Saved {
                    document: saved_document,
                    revision: saved_revision,
                },
            ) if document == self.document
                && document == saved_document
                && revision == saved_revision =>
            {
                self.recovery.clock.saved(document, revision);
                Arc::make_mut(&mut self.view)
                    .workflow
                    .recovery
                    .last_revision = Some(revision);
            }
            (
                Operation::Checkpoint {
                    document,
                    known,
                    restored,
                    ..
                },
                RecoveryValue::Checkpoint { retained },
            ) if document == self.document => {
                if !retained && self.recovery.slot == known {
                    self.recovery.slot = None;
                }
                if self.recovery.restored == restored {
                    self.recovery.restored = None;
                }
            }
            (
                Operation::Leave {
                    document, intent, ..
                },
                RecoveryValue::Cleared,
            ) if document == self.document => {
                self.recovery.allow_leave = Some(document);
                self.recovery.restored = None;
                self.recovery.slot = None;
                self.recovery.retired = true;
                Arc::make_mut(&mut self.view).discard = None;
                if let Err(error) = self.leave(intent, true) {
                    Arc::make_mut(&mut self.view).error = Some(error);
                    self.warn_retained_recovery();
                }
            }
            (
                Operation::Restore { document, .. },
                RecoveryValue::Restored {
                    id,
                    revision,
                    editor,
                    notice,
                },
            ) if document == self.document => {
                self.install(editor, None);
                self.recovery.restored = Some((id, revision));
                let view = Arc::make_mut(&mut self.view);
                view.notice = Some(notice);
                view.workflow.recovery.visible = false;
            }
            (Operation::Discard { id, .. }, RecoveryValue::Cleared) => {
                let view = &mut Arc::make_mut(&mut self.view).workflow.recovery;
                view.entries = view
                    .entries
                    .iter()
                    .filter(|entry| entry.id != id)
                    .cloned()
                    .collect::<Vec<_>>()
                    .into();
                self.scan_recovery();
            }
            _ => {}
        }
    }
}
