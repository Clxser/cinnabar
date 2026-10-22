//! Captured recovery identities survive confirmation and asynchronous list refreshes.

use super::*;

impl Studio {
    /// A discarded document leaves only after its ordered retirement has reached stable storage.
    pub(in crate::menu::studio) fn recovery_leave(
        &mut self,
        intent: DiscardIntent,
    ) -> Result<bool, String> {
        if self.recovery.allow_leave.take() == Some(self.document) {
            return Ok(true);
        }
        if self.recovery.desired.is_some() || Arc::make_mut(&mut self.view).workflow.recovery.busy {
            return Err(
                "Recovery is finishing another operation. Wait before discarding edits.".into(),
            );
        }
        self.transport.pause();
        self.recovery.desired = Some(Operation::Leave {
            document: self.document,
            known: self.recovery.slot.clone(),
            restored: self.recovery.restored.clone(),
            intent,
        });
        Arc::make_mut(&mut self.view).workflow.recovery.busy = true;
        Ok(false)
    }

    /// Resolve display indices before confirmation; later refreshes cannot retarget the intent.
    pub(in crate::menu::studio) fn recovery_restore_intent(
        &self,
        index: usize,
    ) -> Result<DiscardIntent, String> {
        if self.view.workflow.recovery.loading || self.view.workflow.recovery.busy {
            return Err("Recovery is being refreshed. Wait before choosing a copy.".into());
        }
        let entry = self
            .view
            .workflow
            .recovery
            .entries
            .get(index)
            .ok_or("This recovery is no longer listed. Refresh recovery.")?;
        if !entry.recoverable {
            return Err(entry.notice.clone().unwrap_or_else(|| {
                "This recovery cannot be decoded. Preserve its files or discard it explicitly."
                    .into()
            }));
        }
        Ok(DiscardIntent::RestoreRecovery(entry.id.clone()))
    }

    /// Restore uses the captured identity; its worker admits the selected backup again before installation.
    pub(in crate::menu::studio) fn begin_restore(&mut self, id: String) -> Result<(), String> {
        if self.recovery.desired.is_some() || self.view.workflow.recovery.busy {
            return Err("Recovery is busy. Wait for its current operation.".into());
        }
        self.recovery.desired = Some(Operation::Restore {
            document: self.document,
            id,
        });
        let view = Arc::make_mut(&mut self.view);
        view.loading = true;
        view.workflow.recovery.busy = true;
        Ok(())
    }

    /// Captures the selected recovery identity before queueing its explicit discard.
    pub(in crate::menu::studio) fn discard_recovery(&mut self, index: usize) -> Result<(), String> {
        if self.recovery.desired.is_some()
            || self.view.workflow.recovery.busy
            || self.view.workflow.recovery.loading
        {
            return Err("Recovery is busy. Wait for its current operation.".into());
        }
        let entry = self
            .view
            .workflow
            .recovery
            .entries
            .get(index)
            .ok_or("This recovery is no longer listed. Refresh recovery.")?;
        self.recovery.desired = Some(Operation::Discard {
            id: entry.id.clone(),
            revision: entry.discard_through,
        });
        Arc::make_mut(&mut self.view).workflow.recovery.busy = true;
        Ok(())
    }
}
