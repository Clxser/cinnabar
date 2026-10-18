//! Recovery warnings survive unrelated successful scans until their owning failure is resolved.

use super::*;

pub(super) enum Warning {
    Autosave(String),
    Checkpoint(String),
    Retired,
}

impl Warning {
    /// The visible message always describes the unresolved operation, rather than the latest successful scan.
    pub(super) fn message(&self) -> String {
        match self {
            Self::Autosave(message) | Self::Checkpoint(message) => message.clone(),
            Self::Retired => "Recovery for discarded edits was retired. This open copy is retained, but autosave is paused. Retry autosave to protect it in a new recovery slot.".into(),
        }
    }
}

impl Studio {
    /// Keeping the old editor after cancellation or failure does not silently republish intentionally discarded edits.
    pub(in crate::menu::studio) fn warn_retained_recovery(&mut self) {
        if self.recovery.retired && self.editor.as_ref().is_some_and(Editor::dirty) {
            if !matches!(self.recovery.warning, Some(Warning::Checkpoint(_))) {
                self.recovery.warning = Some(Warning::Retired);
            }
            self.publish_recovery_warning();
        }
    }

    /// Only failed writes/checkpoints are persistent warnings; a later scan may resolve its own discovery error.
    pub(super) fn record_recovery_warning(&mut self, operation: &Operation, message: &str) {
        if self.recovery.failed_checkpoint.is_some()
            && matches!(self.recovery.warning, Some(Warning::Checkpoint(_)))
            && matches!(operation, Operation::Begin(_) | Operation::Save(..))
        {
            return;
        }
        let warning = match operation {
            Operation::Begin(_) | Operation::Save(..) => Warning::Autosave(message.into()),
            Operation::Checkpoint { .. } => Warning::Checkpoint(message.into()),
            _ => return,
        };
        self.recovery.warning = Some(warning);
    }

    /// A completed write resolves write failure; a completed cleanup resolves checkpoint failure.
    pub(super) fn resolve_recovery_warning(&mut self, operation: &Operation) {
        let resolved = match (&self.recovery.warning, operation) {
            (Some(Warning::Autosave(_) | Warning::Retired), Operation::Save(document, _)) => {
                *document == self.document && !self.recovery.retired
            }
            (Some(Warning::Checkpoint(_)), Operation::Checkpoint { .. }) => {
                self.recovery.failed_checkpoint.is_none()
            }
            _ => false,
        };
        if resolved {
            self.recovery.warning = None;
        }
        self.publish_recovery_warning();
    }

    /// A retained-editor warning supplements unresolved cleanup instead of hiding its Retry affordance.
    fn publish_recovery_warning(&mut self) {
        let mut message = self.recovery.warning.as_ref().map(Warning::message);
        if self.recovery.retired
            && self.editor.as_ref().is_some_and(Editor::dirty)
            && !matches!(self.recovery.warning, Some(Warning::Retired))
        {
            message = Some(message.map_or_else(
                || Warning::Retired.message(),
                |message| format!("{}\n{message}", Warning::Retired.message()),
            ));
        }
        Arc::make_mut(&mut self.view).workflow.recovery.error = message;
    }
}
