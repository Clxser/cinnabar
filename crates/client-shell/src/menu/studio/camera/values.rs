//! Numeric camera input stays in one bounded draft until an explicit validated commit.

use super::*;

impl Studio {
    /// Focusing the same value preserves typing; a new parameter binds to its exact shot and media time.
    pub(in crate::menu) fn begin_camera_value(
        &mut self,
        parameter: CameraParameter,
    ) -> Result<(), String> {
        let shot = self.camera_shot()?;
        let at = self.transport.position();
        if at < shot.span.start || at > shot.span.end {
            return Err("Move the playhead inside this camera shot before editing a value.".into());
        }
        let revision = self
            .editor
            .as_ref()
            .ok_or("Open a project first.")?
            .revision();
        if self.camera_edit.as_ref().is_some_and(|edit| {
            edit.document == self.document
                && edit.revision == revision
                && edit.shot == shot.id
                && edit.at == at
                && edit.parameter == parameter
        }) {
            return Ok(());
        }
        let value = shot
            .parameter(parameter, at)
            .ok_or("This camera does not own that value.")?;
        self.cancel_title_text();
        self.transport.pause();
        self.camera_text.set_text(&value.to_string());
        self.camera_edit = Some(CameraValueEdit {
            document: self.document,
            revision,
            shot: shot.id,
            at,
            parameter,
        });
        self.publish_camera_value();
        Ok(())
    }

    /// Keystrokes publish presentation text without creating undo entries.
    pub(in crate::menu) fn publish_camera_value(&mut self) {
        if let Some(edit) = &self.camera_edit {
            Arc::make_mut(&mut self.view).camera_draft = Some((
                edit.shot,
                edit.at,
                edit.parameter,
                Arc::from(self.camera_text.as_str()),
            ));
        }
    }

    /// An edited document, different playhead or changed shot cannot receive a stale numeric draft.
    pub(in crate::menu::studio) fn apply_camera_value(&mut self) -> Result<(), String> {
        let edit = self
            .camera_edit
            .as_ref()
            .ok_or("Open a camera value field first.")?;
        let shot = self.camera_shot()?;
        if edit.document != self.document
            || self
                .editor
                .as_ref()
                .is_none_or(|editor| editor.revision() != edit.revision)
            || edit.shot != shot.id
            || edit.at != self.transport.position()
        {
            return Err("This camera or playhead changed while its value was open. Cancel and reopen the value.".into());
        }
        let value = self
            .camera_text
            .as_str()
            .trim()
            .parse::<f64>()
            .map_err(|_| "Enter a finite number using a decimal point.")?;
        let shot = shot
            .with_parameter(edit.at, edit.parameter, value)
            .map_err(|error| error.to_string())?;
        self.edit("Set camera value", &[Command::ReplaceShot { shot }])?;
        self.cancel_camera_value();
        Ok(())
    }

    /// Cancelling restores the saved parameter value without modifying the document.
    pub(in crate::menu) fn cancel_camera_value(&mut self) {
        self.camera_edit = None;
        self.camera_text.set_text("");
        Arc::make_mut(&mut self.view).camera_draft = None;
    }
}
