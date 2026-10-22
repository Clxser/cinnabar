//! Export range drafts bind to one document revision and publish through ordinary undoable settings edits.

use super::*;
use launcher::menu::studio::ExportRangeField;
use recording_studio::export::range::FrameRange;

pub(super) struct RangeEdit {
    document: u64,
    revision: u64,
    field: ExportRangeField,
}

impl Studio {
    /// Native focus derives its available range controls from the same field registry as the painter.
    pub(super) fn export_range_focus_actions(&self) -> Vec<Action> {
        if self.view.loading || self.view.export.phase.busy() {
            return Vec::new();
        }
        let mut actions = vec![Action::FullExportRange];
        if self
            .editor
            .as_ref()
            .is_none_or(|editor| editor.duration() == MediaTime::ZERO)
        {
            return actions;
        }
        for &field in ExportRangeField::ALL {
            actions.extend([
                Action::EditExportRange(field),
                Action::SetExportRangeAtPlayhead(field),
            ]);
        }
        if self.range_edit.is_some() {
            actions.extend([Action::ApplyExportRange, Action::CancelExportRange]);
        }
        actions
    }

    /// Full output clears only the saved export bounds; clip trims, FPS and resolution retain their values.
    pub(super) fn full_export_range(&mut self) -> Result<(), String> {
        let mut settings = self.project()?.export.clone();
        settings.range = None;
        self.edit("Export full project", &[Command::SetExport { settings }])?;
        self.cancel_export_range();
        Ok(())
    }

    /// Playhead bounds use the same output-frame admission as typed seconds.
    pub(super) fn export_range_at_playhead(
        &mut self,
        field: ExportRangeField,
    ) -> Result<(), String> {
        self.set_export_range_value(field, self.transport.position())
    }

    /// Repeated focus preserves typing until its owning document or revision changes.
    pub(in crate::menu) fn begin_export_range(
        &mut self,
        field: ExportRangeField,
    ) -> Result<(), String> {
        if self
            .range_edit
            .as_ref()
            .is_some_and(|edit| edit.field == field && self.range_is_current(edit))
        {
            return Ok(());
        }
        let project = self.project()?;
        let span = project.export.range.unwrap_or(
            Span::new(MediaTime::ZERO, project.duration())
                .map_err(|_| "Add content before setting Export In/Out.")?,
        );
        let revision = self.editor.as_ref().expect("range editor").revision();
        self.cancel_clip_time();
        self.cancel_camera_value();
        self.cancel_title_text();
        self.transport.pause();
        self.range_text
            .set_text(&format!("{:.6}", field.value(span).seconds()));
        self.range_edit = Some(RangeEdit {
            document: self.document,
            revision,
            field,
        });
        self.publish_export_range();
        Ok(())
    }

    /// Keystrokes own only a bounded presentation string; no project mutation or IO occurs here.
    pub(in crate::menu) fn publish_export_range(&mut self) {
        if let Some(edit) = &self.range_edit {
            Arc::make_mut(&mut self.view).workflow.export_range_draft =
                Some((edit.field, Arc::from(self.range_text.as_str())));
        }
    }

    /// Applying a stale field cannot change a newly loaded project or overwrite intervening edits.
    pub(super) fn apply_export_range(&mut self) -> Result<(), String> {
        let edit = self
            .range_edit
            .as_ref()
            .ok_or("Open Export In or Export Out first.")?;
        if !self.range_is_current(edit) {
            return Err("This project changed. Cancel and reopen Export In/Out.".into());
        }
        let field = edit.field;
        let seconds = self
            .range_text
            .as_str()
            .trim()
            .parse::<f64>()
            .map_err(|_| "Enter finite project timeline seconds with a decimal point.")?;
        let time = MediaTime::from_seconds(seconds)
            .map_err(|_| "Export time must be finite, nonnegative and within seven days.")?;
        self.set_export_range_value(field, time)?;
        self.cancel_export_range();
        Ok(())
    }

    /// Range settings commit atomically on the selected output frame grid; Out is an exclusive bound.
    fn set_export_range_value(
        &mut self,
        field: ExportRangeField,
        value: MediaTime,
    ) -> Result<(), String> {
        let project = self.project()?;
        let duration = project.duration();
        if value > duration {
            return Err(format!(
                "Export time must be at most {:.6} project seconds.",
                duration.seconds()
            ));
        }
        let mut settings = project.export.clone();
        let mut range = settings.range.unwrap_or(
            Span::new(MediaTime::ZERO, duration)
                .map_err(|_| "Add content before setting Export In/Out.")?,
        );
        let value = settings
            .fps
            .snap(value)
            .map_err(|error| error.to_string())?
            .min(duration);
        match field {
            ExportRangeField::In => range.start = value,
            ExportRangeField::Out => range.end = value,
        }
        FrameRange::new(settings.fps, duration, Some(range)).map_err(|error| error.to_string())?;
        settings.range = Some(range);
        self.edit("Set export range", &[Command::SetExport { settings }])
    }

    /// Cancellation and document replacement restore saved range values without an undo entry.
    pub(in crate::menu) fn cancel_export_range(&mut self) {
        self.range_edit = None;
        self.range_text.set_text("");
        Arc::make_mut(&mut self.view).workflow.export_range_draft = None;
    }

    /// An immutable revision change invalidates the old draft before it can be applied.
    pub(super) fn reconcile_export_range(&mut self) {
        if self
            .range_edit
            .as_ref()
            .is_some_and(|edit| !self.range_is_current(edit))
        {
            self.cancel_export_range();
        }
    }

    /// Export fields belong to a project revision rather than any selected source clip.
    fn range_is_current(&self, edit: &RangeEdit) -> bool {
        edit.document == self.document
            && self
                .editor
                .as_ref()
                .is_some_and(|editor| editor.revision() == edit.revision)
    }
}

#[cfg(test)]
mod tests;
