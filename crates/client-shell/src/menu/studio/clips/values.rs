//! A typed clip time remains a guarded presentation draft until one ordinary edit is applied.

use super::*;
use launcher::menu::studio::ClipTimeField;

impl Studio {
    /// Repeated focus preserves typing; another field starts from the selected clip's saved clock.
    pub(in crate::menu) fn begin_clip_time(&mut self, field: ClipTimeField) -> Result<(), String> {
        let clip = self.selected_visual()?;
        if self
            .clip_edit
            .as_ref()
            .is_some_and(|edit| edit.field == field && self.clip_time_is_current(edit))
        {
            return Ok(());
        }
        let revision = self
            .editor
            .as_ref()
            .ok_or("Open a project first.")?
            .revision();
        self.cancel_title_text();
        self.cancel_camera_value();
        self.transport.pause();
        self.clip_text
            .set_text(&format!("{:.6}", field.value(&clip).seconds()));
        self.clip_edit = Some(ClipTimeEdit {
            document: self.document,
            revision,
            clip: clip.id,
            field,
        });
        self.publish_clip_time();
        Ok(())
    }

    /// Keystrokes update the visible draft without touching the project or undo history.
    pub(in crate::menu) fn publish_clip_time(&mut self) {
        if let Some(edit) = &self.clip_edit {
            Arc::make_mut(&mut self.view).clip_draft =
                Some((edit.clip, edit.field, Arc::from(self.clip_text.as_str())));
        }
    }

    /// Selection and project publications discard a draft as soon as its original owner changes.
    pub(in crate::menu::studio) fn reconcile_clip_time(&mut self) {
        if self
            .clip_edit
            .as_ref()
            .is_some_and(|edit| !self.clip_time_is_current(edit))
        {
            self.cancel_clip_time();
        }
    }

    /// Position moves the clip; source handles preserve that position and let ordinary trim own ripple and links.
    pub(in crate::menu::studio) fn apply_clip_time(&mut self) -> Result<(), String> {
        let edit = self
            .clip_edit
            .as_ref()
            .ok_or("Open a clip time field first.")?;
        if !self.clip_time_is_current(edit) {
            return Err("This clip or project changed while its time was open. Cancel and reopen the field.".into());
        }
        let clip = self.selected_visual()?;
        let seconds = self
            .clip_text
            .as_str()
            .trim()
            .parse::<f64>()
            .map_err(|_| "Enter finite seconds using a decimal point.")?;
        let at = MediaTime::from_seconds(seconds).map_err(|_| {
            format!(
                "Enter finite seconds between 0 and {}.",
                MediaTime(recording_studio::time::MAX_MEDIA_MICROS).seconds(),
            )
        })?;
        let command = match edit.field {
            ClipTimeField::Position => Command::MoveClip {
                id: clip.id,
                at,
                track: clip.track,
            },
            ClipTimeField::SourceIn | ClipTimeField::SourceOut => {
                let source = self
                    .project()?
                    .sources
                    .iter()
                    .find(|source| source.id == clip.source)
                    .ok_or("The selected clip's source is missing.")?;
                let maximum = if source.kind == MediaKind::Image {
                    MediaTime(recording_studio::time::MAX_MEDIA_MICROS)
                } else {
                    source.duration
                };
                if at > maximum {
                    return Err(format!(
                        "Source time must be at most {:.6} seconds.",
                        maximum.seconds()
                    ));
                }
                let (start, end) = match edit.field {
                    ClipTimeField::SourceIn => (at, clip.source_range.end),
                    ClipTimeField::SourceOut => (clip.source_range.start, at),
                    ClipTimeField::Position => unreachable!(),
                };
                let source = Span::new(start, end)
                    .map_err(|_| "Source In must be earlier than Source Out.".to_owned())?;
                Command::TrimClip {
                    id: clip.id,
                    source,
                    ripple: self.view.workspace.ripple,
                }
            }
        };
        self.edit("Set clip time", &[command])?;
        self.cancel_clip_time();
        Ok(())
    }

    /// Cancelling restores saved presentation values without creating an undo entry.
    pub(in crate::menu) fn cancel_clip_time(&mut self) {
        self.clip_edit = None;
        self.clip_text.set_text("");
        Arc::make_mut(&mut self.view).clip_draft = None;
    }

    /// A draft is bound to one selected clip in one exact document revision.
    fn clip_time_is_current(&self, edit: &ClipTimeEdit) -> bool {
        edit.document == self.document
            && self.view.selected == Some(edit.clip)
            && self
                .editor
                .as_ref()
                .is_some_and(|editor| editor.revision() == edit.revision)
    }
}
