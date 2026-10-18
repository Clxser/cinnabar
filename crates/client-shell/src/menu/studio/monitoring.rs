//! Viewer monitoring changes runtime clocks only; no command enters the saved project undo history.

use super::*;
use launcher::menu::studio::monitoring::MonitoringAction;
use recording_studio::playback::monitoring::{MonitoringSpeed, jumped, parse_seek, seek_text};

impl Studio {
    /// Explicit transport intents preserve the document and stop camera/text gestures before seeking.
    pub(super) fn monitoring_action(&mut self, action: MonitoringAction) -> Result<(), String> {
        let duration = self.project()?.duration();
        match action {
            MonitoringAction::EditSeek => return self.begin_monitor_seek(),
            MonitoringAction::ApplySeek => return self.apply_monitor_seek(),
            MonitoringAction::CancelSeek => {
                self.cancel_monitor_seek();
                return Ok(());
            }
            MonitoringAction::Speed(speed) => self
                .transport
                .set_rate(speed.rate())
                .map_err(|e| e.to_string())?,
            MonitoringAction::StepSpeed(step) => {
                let speed = self.view.workspace.monitoring.speed.stepped(step);
                self.transport
                    .set_rate(speed.rate())
                    .map_err(|e| e.to_string())?;
            }
            MonitoringAction::ToggleLoop => {
                let span = if self.transport.looping().is_some() {
                    None
                } else {
                    Some(Span::new(MediaTime::ZERO, duration).map_err(|e| e.to_string())?)
                };
                self.transport
                    .set_loop(span, duration)
                    .map_err(|e| e.to_string())?;
            }
            MonitoringAction::Jump(seconds) => self.transport.seek(
                jumped(self.transport.position(), seconds, duration),
                duration,
            ),
            MonitoringAction::Start => self.transport.seek(MediaTime::ZERO, duration),
            MonitoringAction::End => {
                self.transport.pause();
                self.transport.seek(duration, duration);
            }
        }
        self.cancel_monitor_seek();
        self.cancel_camera_value();
        self.cancel_clip_time();
        self.release_camera();
        self.publish_monitoring();
        Ok(())
    }

    /// Seeking drafts belong to the current document revision; another publication invalidates them.
    pub(in crate::menu) fn begin_monitor_seek(&mut self) -> Result<(), String> {
        let editor = self.editor.as_ref().ok_or("Open a project first.")?;
        let identity = (self.document, editor.revision());
        if self.seek_edit == Some(identity) {
            return Ok(());
        }
        self.cancel_camera_value();
        self.cancel_clip_time();
        self.cancel_title_text();
        self.release_camera();
        self.transport.pause();
        self.seek_text
            .set_text(&seek_text(self.transport.position()));
        self.seek_edit = Some(identity);
        self.publish_monitor_seek();
        Ok(())
    }

    /// Text edits change only the visible runtime draft.
    pub(in crate::menu) fn publish_monitor_seek(&mut self) {
        if self.seek_edit.is_some() {
            Arc::make_mut(&mut self.view)
                .workspace
                .monitoring
                .seek_draft = Some(Arc::from(self.seek_text.as_str()));
        }
    }

    /// Invalid values stay editable and never seek, dirty the project or mutate source media.
    fn apply_monitor_seek(&mut self) -> Result<(), String> {
        let editor = self.editor.as_ref().ok_or("Open a project first.")?;
        if self.seek_edit != Some((self.document, editor.revision())) {
            return Err("The document changed. Reopen the seek field.".into());
        }
        let duration = editor.duration();
        let at = parse_seek(self.seek_text.as_str()).map_err(|e| e.to_string())?;
        if at > duration {
            return Err("The seek time is beyond the project end.".into());
        }
        self.transport.pause();
        self.transport.seek(at, duration);
        self.cancel_monitor_seek();
        Ok(())
    }

    /// Discarding a typed seek leaves the exact current transport position intact.
    pub(in crate::menu) fn cancel_monitor_seek(&mut self) {
        self.seek_edit = None;
        self.seek_text.set_text("");
        Arc::make_mut(&mut self.view)
            .workspace
            .monitoring
            .seek_draft = None;
    }

    /// A changed project duration updates a whole-project monitor loop without creating an export range.
    pub(super) fn publish_monitoring(&mut self) {
        if self.seek_edit.is_some_and(|identity| {
            self.editor
                .as_ref()
                .is_none_or(|editor| identity != (self.document, editor.revision()))
        }) {
            self.cancel_monitor_seek();
        }
        if let Some(editor) = &self.editor {
            let duration = editor.duration();
            if self
                .transport
                .looping()
                .is_some_and(|span| span.end != duration)
            {
                let span = Span::new(MediaTime::ZERO, duration).ok();
                let _ = self.transport.set_loop(span, duration);
            }
        }
        let workspace = &mut Arc::make_mut(&mut self.view).workspace.monitoring;
        workspace.speed = MonitoringSpeed::ALL
            .iter()
            .copied()
            .find(|s| s.rate() == self.transport.rate())
            .unwrap_or_default();
        workspace.looping = self.transport.looping().is_some();
    }

    /// Fallback keyboard/controller focus exposes the same monitoring commands as the transport.
    pub(super) fn monitoring_focus_actions(&self) -> Vec<Action> {
        let mut actions = vec![
            MonitoringAction::Start,
            MonitoringAction::Jump(-recording_studio::playback::monitoring::JUMP_SECONDS),
            MonitoringAction::Jump(recording_studio::playback::monitoring::JUMP_SECONDS),
            MonitoringAction::End,
            MonitoringAction::StepSpeed(-1),
            MonitoringAction::StepSpeed(1),
            MonitoringAction::Speed(MonitoringSpeed::Normal),
            MonitoringAction::ToggleLoop,
            MonitoringAction::EditSeek,
        ];
        if self.seek_edit.is_some() {
            actions.extend([MonitoringAction::ApplySeek, MonitoringAction::CancelSeek]);
        }
        actions.into_iter().map(Action::Monitoring).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Image metadata provides a real timeline without touching files, workers or renderer state.
    fn studio() -> Studio {
        let mut project = Project::new("Monitoring");
        let duration = MediaTime(10_000_000);
        project.sources.push(MediaSource {
            id: Id(1),
            name: "Still".into(),
            path: "unused.png".into(),
            kind: MediaKind::Image,
            duration,
            size: [64, 64],
            frame_rate: None,
            sha256: None,
        });
        project.clips.push(Clip {
            id: Id(2),
            source: Id(1),
            source_range: Span::new(MediaTime::ZERO, duration).unwrap(),
            at: MediaTime::ZERO,
            rate: PlaybackRate::default(),
            speed_ramp: None,
            track: 0,
            framing: Framing::default(),
            enabled: true,
        });
        let mut studio = Studio::new(PathBuf::new());
        studio.install(Editor::new(project).unwrap(), None);
        studio
    }
    #[test]
    fn monitoring_controls_preserve_saved_timing_undo_state_and_source_metadata() {
        let mut studio = studio();
        let project = studio.editor.as_ref().unwrap().project().clone();
        let before = project.to_json().unwrap();
        let revision = studio.editor.as_ref().unwrap().revision();
        studio.transport.play(project.duration());
        let epoch = studio.transport.epoch();
        studio.activate(Action::Monitoring(MonitoringAction::Speed(
            MonitoringSpeed::Eight,
        )));
        assert_eq!(studio.transport.rate(), MonitoringSpeed::Eight.rate());
        assert_ne!(studio.transport.epoch(), epoch);
        assert!(studio.transport.playing());
        studio.activate(Action::Monitoring(MonitoringAction::Jump(5)));
        assert_eq!(studio.transport.position(), MediaTime(5_000_000));
        studio.activate(Action::Monitoring(MonitoringAction::ToggleLoop));
        assert!(studio.transport.looping().is_some());
        studio.activate(Action::Monitoring(MonitoringAction::End));
        assert!(!studio.transport.playing());
        assert_eq!(studio.transport.position(), project.duration());
        assert_eq!(studio.editor.as_ref().unwrap().revision(), revision);
        assert!(Arc::ptr_eq(
            studio.editor.as_ref().unwrap().project(),
            &project
        ));
        assert_eq!(project.to_json().unwrap(), before);
    }
    #[test]
    fn typed_seek_preserves_microseconds_and_keeps_invalid_drafts_editable() {
        let mut studio = studio();
        studio.begin_monitor_seek().unwrap();
        studio.seek_text.set_text("00:00:03.123456");
        studio.apply_monitor_seek().unwrap();
        assert_eq!(studio.transport.position(), MediaTime(3_123_456));
        assert!(studio.view.workspace.monitoring.seek_draft.is_none());
        studio.begin_monitor_seek().unwrap();
        studio.seek_text.set_text("99:00");
        assert!(studio.apply_monitor_seek().is_err());
        assert!(studio.seek_edit.is_some());
        studio.cancel_monitor_seek();
        assert_eq!(studio.transport.position(), MediaTime(3_123_456));
        studio.begin_monitor_seek().unwrap();
        studio.document += 1;
        assert!(studio.apply_monitor_seek().is_err());
        studio.publish_monitoring();
        assert!(studio.seek_edit.is_none());
    }
}
