//! Recovery has its own bounded storage worker, so thumbnail/probe jobs cannot starve periodic snapshots.

use super::*;
use recording_studio::storage::{RecoveryRequest, RecoveryValue};

mod clock;
mod completion;
mod dispatch;
mod intents;
mod status;
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct Recovery {
    initial_scan: bool,
    service: Option<ProjectStorage>,
    pending: Option<(u64, Operation)>,
    desired: Option<Operation>,
    checkpoint: Option<Operation>,
    scan: bool,
    slot: Option<String>,
    restored: Option<(String, u64)>,
    clock: clock::Clock,
    allow_leave: Option<u64>,
    failed_checkpoint: Option<Operation>,
    retired: bool,
    warning: Option<status::Warning>,
}

#[derive(Clone)]
enum Operation {
    Scan,
    Begin(u64),
    Save(u64, u64),
    Checkpoint {
        document: u64,
        revision: u64,
        known: Option<String>,
        restored: Option<(String, u64)>,
    },
    Leave {
        document: u64,
        known: Option<String>,
        restored: Option<(String, u64)>,
        intent: DiscardIntent,
    },
    Restore {
        document: u64,
        id: String,
    },
    Discard {
        id: String,
        revision: u64,
    },
}

impl Studio {
    /// First entry discovers prior-session copies without opening files or repeatedly scanning during playback.
    pub(super) fn prime_recovery_scan(&mut self) {
        if !std::mem::replace(&mut self.recovery.initial_scan, true) {
            self.scan_recovery();
        }
    }

    /// Scanning is an explicit asynchronous route and preserves the editor behind the recovery list.
    pub(super) fn show_recovery(&mut self) {
        self.transport.pause();
        self.release_camera();
        Arc::make_mut(&mut self.view).workflow.recovery.visible = true;
        self.scan_recovery();
    }

    pub(super) fn scan_recovery(&mut self) {
        self.recovery.scan = true;
        Arc::make_mut(&mut self.view).workflow.recovery.loading = true;
    }

    /// Saving an older snapshot never clears a newer edited autosave.
    pub(super) fn recovery_saved(&mut self, revision: u64) {
        self.recovery.checkpoint = Some(Operation::Checkpoint {
            document: self.document,
            revision,
            known: self.recovery.slot.clone(),
            restored: self.recovery.restored.clone(),
        });
    }

    pub(super) fn reset_recovery_document(&mut self) {
        self.recovery.slot = None;
        self.recovery.restored = None;
        self.recovery.retired = false;
        if !matches!(self.recovery.warning, Some(status::Warning::Checkpoint(_))) {
            self.recovery.warning = None;
        }
        self.recovery.clock.reset(self.document);
        let recovery = &mut Arc::make_mut(&mut self.view).workflow.recovery;
        recovery.last_revision = None;
        recovery.error = self.recovery.warning.as_ref().map(status::Warning::message);
    }

    /// Explicitly keeping a retired editor starts a new generation without replacing its metadata or pending save.
    pub(super) fn retry_autosave(&mut self) {
        if self.recovery.retired {
            let previous = self.document;
            self.document = self.document.wrapping_add(1);
            if let Some(save) = self
                .save_job
                .as_mut()
                .filter(|save| save.document == previous)
            {
                save.document = self.document;
            }
            self.recovery.slot = None;
            self.recovery.retired = false;
            self.recovery.clock.reset(self.document);
        }
        if let Some(operation) = self.recovery.failed_checkpoint.take() {
            self.recovery.checkpoint = Some(operation);
        }
        self.recovery.clock.retry();
        self.recovery.scan = true;
        Arc::make_mut(&mut self.view).workflow.recovery.error =
            self.recovery.warning.as_ref().map(status::Warning::message);
    }

    /// Cancelling restore invalidates its job identity without waiting for a bounded metadata read.
    pub(super) fn cancel_recovery_restore(&mut self) {
        if self
            .recovery
            .pending
            .as_ref()
            .is_some_and(|(_, operation)| matches!(operation, Operation::Restore { .. }))
        {
            self.recovery.pending = None;
        }
        if self
            .recovery
            .desired
            .as_ref()
            .is_some_and(|operation| matches!(operation, Operation::Restore { .. }))
        {
            self.recovery.desired = None;
        }
        let view = Arc::make_mut(&mut self.view);
        view.loading = false;
        view.workflow.recovery.busy = false;
        self.warn_retained_recovery();
    }

    /// A caller-owned delta drives the timer even while the editor is hidden; no wall clock or IO enters this path.
    pub(super) fn tick_recovery(&mut self, elapsed: f64) {
        let completed = self.recovery.service.as_ref().map(ProjectStorage::poll);
        match completed {
            Some(Ok(Some(result))) => self.recovery_completed(result),
            Some(Err(error)) => {
                self.recovery.service = None;
                let failed = self.recovery.pending.take().map(|(_, operation)| operation);
                if matches!(failed, Some(Operation::Checkpoint { .. })) {
                    self.recovery.failed_checkpoint = failed.clone();
                }
                let message = format!(
                    "Autosave worker stopped: {error}. Your open editor is intact. Retry autosave or save the project explicitly."
                );
                if let Some(operation) = &failed {
                    self.record_recovery_warning(operation, &message);
                }
                let snapshot = Arc::make_mut(&mut self.view);
                if matches!(failed, Some(Operation::Restore { .. })) {
                    snapshot.loading = false;
                }
                let view = &mut snapshot.workflow.recovery;
                view.loading = self.recovery.scan;
                view.busy = false;
                view.autosaving = false;
                view.error = Some(message);
                self.warn_retained_recovery();
            }
            _ => {}
        }
        let due = self.editor.as_ref().is_some_and(|editor| {
            self.recovery
                .clock
                .advance(self.document, editor.revision(), editor.dirty(), elapsed)
        });
        if self.recovery.pending.is_some() {
            return;
        }
        let operation = if let Some(operation) = self.recovery.desired.take() {
            Some(operation)
        } else if let Some(operation) = self.recovery.checkpoint.take() {
            Some(operation)
        } else if self.recovery.scan {
            self.recovery.scan = false;
            Some(Operation::Scan)
        } else if due && !self.recovery.retired && self.view.discard.is_none() && !self.view.loading
        {
            Some(if self.recovery.slot.is_none() {
                Operation::Begin(self.document)
            } else {
                Operation::Save(
                    self.document,
                    self.editor.as_ref().expect("autosave editor").revision(),
                )
            })
        } else {
            None
        };
        if let Some(mut operation) = operation {
            if let Operation::Checkpoint {
                document, known, ..
            }
            | Operation::Leave {
                document, known, ..
            } = &mut operation
            {
                if *document == self.document && known.is_none() {
                    *known = self.recovery.slot.clone();
                }
            }
            let retry =
                matches!(operation, Operation::Checkpoint { .. }).then(|| operation.clone());
            let restoring = matches!(operation, Operation::Restore { .. });
            if let Err(error) = self.dispatch_recovery(operation.clone()) {
                self.recovery.failed_checkpoint = retry;
                self.recovery.clock.submitted();
                self.record_recovery_warning(&operation, &error);
                let snapshot = Arc::make_mut(&mut self.view);
                if restoring {
                    snapshot.loading = false;
                }
                let view = &mut snapshot.workflow.recovery;
                view.loading = self.recovery.scan;
                view.busy = false;
                view.autosaving = false;
                view.error = Some(error);
                self.warn_retained_recovery();
            }
        }
    }
}
