//! Native title edits are reversible metadata transactions; typing stays in a bounded explicit draft.

use recording_studio::title::TitleParameter;

use super::*;

impl Studio {
    /// Reads the selected title without cloning tracks or accidentally selecting a visual clip.
    fn selected_title(&self) -> Result<&TitleClip, String> {
        self.project()?
            .titles
            .iter()
            .find(|title| Some(title.id) == self.view.selected)
            .ok_or_else(|| "Select a title on the timeline first.".into())
    }

    pub(super) fn title_focus_actions(&self) -> Vec<Action> {
        let Ok(title) = self.selected_title() else {
            return Vec::new();
        };
        let mut actions = vec![
            Action::EditTitleText,
            Action::CycleTitleStyle,
            Action::ToggleTitleShadow,
            Action::ToggleCameraFine,
            Action::TrimTitleStart,
            Action::TrimTitleEnd,
            Action::MoveTitle(-1),
            Action::MoveTitle(1),
        ];
        if self
            .view
            .title_draft
            .as_ref()
            .is_some_and(|(id, _)| *id == title.id)
        {
            actions.extend([Action::ApplyTitleText, Action::CancelTitleText]);
        }
        for &parameter in TitleParameter::ALL {
            actions.extend([-1, 1].map(|direction| Action::AdjustTitle {
                parameter,
                direction,
            }));
        }
        actions
    }

    /// Re-entering the same field preserves selection and uncommitted typing; a new target gets a new draft.
    pub(in crate::menu) fn begin_title_text(&mut self) -> Result<(), String> {
        let title = self.selected_title()?;
        if self
            .title_edit
            .as_ref()
            .is_some_and(|(document, id, _)| *document == self.document && *id == title.id)
        {
            return Ok(());
        }
        let (id, text) = (title.id, title.text.clone());
        self.title_text.set_text(&text);
        self.title_edit = Some((self.document, id, text));
        self.publish_title_text();
        Ok(())
    }

    /// Text/caret input never grows the undo history per keystroke; Apply commits one complete edit.
    pub(in crate::menu) fn publish_title_text(&mut self) {
        if let Some((_, id, _)) = &self.title_edit {
            Arc::make_mut(&mut self.view).title_draft =
                Some((*id, Arc::from(self.title_text.as_str())));
        }
    }

    pub(super) fn apply_title_text(&mut self) -> Result<(), String> {
        let (document, id, before) = self
            .title_edit
            .as_ref()
            .ok_or("Open a title text field first.")?;
        let mut title = self.selected_title()?.clone();
        if *document != self.document || *id != title.id || *before != title.text {
            return Err("This title changed while its text draft was open. Cancel and reopen the text field.".into());
        }
        let text = self.title_text.as_str();
        if title.text != text {
            title.text = text.to_owned();
            self.edit("Change title text", &[Command::ReplaceTitle { title }])?;
        }
        self.cancel_title_text();
        Ok(())
    }

    pub(super) fn cancel_title_text(&mut self) {
        self.title_edit = None;
        self.title_text.set_text("");
        Arc::make_mut(&mut self.view).title_draft = None;
    }

    /// Both persisted font roles map to the same installed faces used by ordinary native UI.
    pub(super) fn cycle_title_style(&mut self) -> Result<(), String> {
        let mut title = self.selected_title()?.clone();
        title.style = match title.style {
            TitleStyle::Body => TitleStyle::Heading,
            TitleStyle::Heading => TitleStyle::Body,
        };
        self.edit("Change title font", &[Command::ReplaceTitle { title }])
    }

    /// Toggles ink shadow without changing text, duration or fades.
    pub(super) fn toggle_title_shadow(&mut self) -> Result<(), String> {
        let mut title = self.selected_title()?.clone();
        title.shadow = !title.shadow;
        self.edit("Change title shadow", &[Command::ReplaceTitle { title }])
    }

    /// Domain bounds and steps are shared with the parameter labels shown in the inspector.
    pub(super) fn adjust_title(
        &mut self,
        parameter: TitleParameter,
        direction: i8,
    ) -> Result<(), String> {
        let mut title = self.selected_title()?.clone();
        parameter.adjust(&mut title, direction, self.view.camera_fine);
        self.edit("Adjust title", &[Command::ReplaceTitle { title }])
    }

    /// Trimming snaps to an output frame and clamps both fades inside the shortened title.
    pub(super) fn trim_title(&mut self, start: bool) -> Result<(), String> {
        let mut title = self.selected_title()?.clone();
        let at = self
            .project()?
            .export
            .fps
            .snap(self.transport.position())
            .map_err(|e| e.to_string())?;
        title.span = if start {
            Span::new(at, title.span.end)
        } else {
            Span::new(title.span.start, at)
        }
        .map_err(|e| e.to_string())?;
        title.fade_in = title.fade_in.min(title.span.duration());
        title.fade_out = title.fade_out.min(title.span.duration());
        self.edit("Trim title", &[Command::ReplaceTitle { title }])
    }

    /// Moves a whole title by output frames, preserving duration and opacity envelopes.
    pub(super) fn move_title(&mut self, frames: i8) -> Result<(), String> {
        let mut title = self.selected_title()?.clone();
        let fps = self.project()?.export.fps;
        let frame = fps.frame_at(title.span.start).map_err(|e| e.to_string())?;
        let start = fps
            .frame_time(frame.saturating_add_signed(i64::from(frames)))
            .map_err(|e| e.to_string())?;
        title.span = Span::new(
            start,
            start
                .checked_add(title.span.duration())
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        self.edit("Move title", &[Command::ReplaceTitle { title }])
    }
}
