//! Reversible export controls and worker-persisted live recording preferences.

use super::*;
use recording_studio::{
    model::{Encoder, ExportCodec},
    settings,
};

impl Studio {
    /// The FPS preference affects the next take only; an active capture keeps its admitted clock.
    pub(super) fn cycle_capture_fps(&mut self) -> Result<(), String> {
        if self.view.capture.phase.busy() {
            return Err("Stop recording before changing capture FPS.".into());
        }
        let mut settings = self.view.workflow.capture_settings;
        settings.fps = settings::next_fps(settings.fps);
        self.store_capture_settings(settings)
    }

    /// Capture size is frozen for a take so encoded frames cannot change dimensions midway.
    pub(super) fn cycle_capture_size(&mut self) -> Result<(), String> {
        if self.view.capture.phase.busy() {
            return Err("Stop recording before changing capture size.".into());
        }
        let mut settings = self.view.workflow.capture_settings;
        settings.size = settings.size.next();
        self.store_capture_settings(settings)
    }

    /// Prepared hand geometry is opt-in for the next take and remains distinct from POV video pixels.
    pub(super) fn toggle_capture_hand(&mut self) -> Result<(), String> {
        if self.view.capture.phase.busy() {
            return Err("Stop recording before changing hand capture.".into());
        }
        let mut settings = self.view.workflow.capture_settings;
        settings.hand = !settings.hand;
        self.store_capture_settings(settings)
    }

    /// Recording personal HUD and chat pixels is explicitly opt-in for each saved preference.
    pub(super) fn toggle_capture_hud(&mut self) -> Result<(), String> {
        if self.view.capture.phase.busy() {
            return Err("Stop recording before changing HUD capture.".into());
        }
        let mut settings = self.view.workflow.capture_settings;
        settings.hud = !settings.hud;
        self.store_capture_settings(settings)
    }

    /// One outstanding preferences write prevents older asynchronous completions overwriting new choices.
    fn store_capture_settings(
        &mut self,
        settings: recording_studio::capture::CaptureSettings,
    ) -> Result<(), String> {
        if self.preferences_job.is_some() {
            return Err("Capture preferences are being saved or loaded. Try again when the current operation finishes.".into());
        }
        self.ensure_storage()?;
        self.thumbnails_cancel.cancel();
        self.preferences_job = Some(
            self.storage
                .as_mut()
                .expect("storage exists")
                .request(StorageRequest::SaveCaptureSettings {
                    root: self.root.clone(),
                    settings,
                })
                .map_err(|error| error.to_string())?,
        );
        self.preferences_changed = true;
        Arc::make_mut(&mut self.view).workflow.capture_settings = settings;
        Ok(())
    }

    /// Every project output setting enters the same undo history as ordinary timeline edits.
    pub(super) fn output_setting(&mut self, action: Action) -> Result<(), String> {
        let mut settings = self.project()?.export.clone();
        match action {
            Action::CycleExportFps => settings.fps = settings::next_fps(settings.fps),
            Action::CycleExportSize => {
                let index = settings::SIZE_PRESETS
                    .iter()
                    .position(|&size| size == settings.size)
                    .map_or(0, |index| (index + 1) % settings::SIZE_PRESETS.len());
                settings.size = settings::SIZE_PRESETS[index];
            }
            Action::CycleExportCodec => {
                settings.codec = match settings.codec {
                    ExportCodec::H264 => ExportCodec::Hevc,
                    ExportCodec::Hevc => ExportCodec::Av1,
                    ExportCodec::Av1 => ExportCodec::H264,
                }
            }
            Action::CycleExportEncoder => {
                settings.encoder = match settings.encoder {
                    Encoder::Auto => Encoder::Cpu,
                    Encoder::Cpu => Encoder::Nvidia,
                    Encoder::Nvidia => Encoder::Auto,
                }
            }
            Action::AdjustExportQuality(direction) => {
                settings.quality = settings.quality.saturating_add_signed(direction).min(51)
            }
            Action::ToggleExportAudio => settings.include_audio = !settings.include_audio,
            _ => return Err("This output control is unavailable.".into()),
        }
        self.edit("Change export settings", &[Command::SetExport { settings }])
    }

    /// A path draft uses the regular native text editor and performs no filesystem work while typing.
    pub(in crate::menu) fn begin_destination(&mut self) {
        if self.view.workflow.destination_draft.is_some() {
            return;
        }
        let path = self
            .view
            .workflow
            .destination
            .clone()
            .unwrap_or_else(|| self.root.join("exports"));
        self.destination_text.set_text(&path.display().to_string());
        self.publish_destination();
    }

    /// Publishing one bounded path keeps text input independent of worker IO and project revisions.
    pub(in crate::menu) fn publish_destination(&mut self) {
        Arc::make_mut(&mut self.view).workflow.destination_draft =
            Some(Arc::from(self.destination_text.as_str()));
    }

    /// An MP4 filename selects one no-clobber file; other paths select a folder with generated names.
    pub(super) fn apply_destination(&mut self) -> Result<(), String> {
        let text = self.destination_text.as_str().trim();
        if text.is_empty() || text.chars().any(char::is_control) {
            return Err("Enter a destination folder or an MP4 filename.".into());
        }
        let path = PathBuf::from(text);
        if !path.is_absolute() {
            return Err(
                "Enter an absolute output path, including its drive or root folder.".into(),
            );
        }
        let workflow = &mut Arc::make_mut(&mut self.view).workflow;
        workflow.destination = Some(path);
        workflow.destination_draft = None;
        Ok(())
    }

    pub(super) fn workflow_focus_actions(&self) -> Vec<Action> {
        let mut actions = vec![Action::ToggleSettings];
        if self.view.workflow.media_busy {
            actions.push(Action::CancelMedia);
        } else {
            actions.push(Action::ImportMedia);
            if self.view.selected.is_some() {
                actions.push(Action::RelinkMedia);
            }
        }
        if self.view.workflow.show_settings {
            if !self.view.capture.phase.busy() {
                actions.extend([
                    Action::CycleCaptureFps,
                    Action::CycleCaptureSize,
                    Action::ToggleCaptureHand,
                    Action::ToggleCaptureHud,
                ]);
            }
            if self.editor.is_some() {
                actions.extend([
                    Action::CycleExportFps,
                    Action::CycleExportSize,
                    Action::CycleExportCodec,
                    Action::CycleExportEncoder,
                    Action::AdjustExportQuality(-1),
                    Action::AdjustExportQuality(1),
                    Action::ToggleExportAudio,
                    Action::EditExportDestination,
                ]);
                if self.view.workflow.destination_draft.is_some() {
                    actions.extend([
                        Action::ApplyExportDestination,
                        Action::CancelExportDestination,
                    ]);
                }
            }
        }
        actions
    }
}
