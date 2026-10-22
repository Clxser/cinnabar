//! The device sample cursor paces audible playback; video backpressure can stall both clocks together.

use super::*;
use recording_studio::audio::{PreviewRequest, PreviewStatus};

mod pause;
pub(super) use pause::PauseSync;

impl Studio {
    /// Audio selection never silently edits a neighbouring visual clip or detaches linked game sound.
    fn selected_audio(&self) -> Result<recording_studio::model::AudioClip, String> {
        self.project()?
            .audio
            .iter()
            .find(|clip| Some(clip.id) == self.view.selected)
            .cloned()
            .ok_or_else(|| "Select an audio clip first.".into())
    }

    /// Clip mute is independent of track mute and remains an ordinary reversible mix edit.
    pub(super) fn toggle_audio_mute(&mut self) -> Result<(), String> {
        let mut audio = self.selected_audio()?;
        audio.muted = !audio.muted;
        self.edit("Toggle audio mute", &[Command::ReplaceAudio { audio }])
    }

    /// Shared domain bounds own gain and output-time fades; source timing and linkage stay intact.
    pub(super) fn adjust_audio(
        &mut self,
        parameter: recording_studio::audio::AudioParameter,
        direction: i8,
    ) -> Result<(), String> {
        let audio = parameter
            .adjusted(&self.selected_audio()?, direction)
            .map_err(|error| error.to_string())?;
        self.edit("Adjust audio mix", &[Command::ReplaceAudio { audio }])
    }

    /// Native focus fallback derives the same selected-audio intents as the painted inspector.
    pub(super) fn audio_focus_actions(&self) -> Vec<Action> {
        if !self.project().is_ok_and(|project| {
            project
                .audio
                .iter()
                .any(|clip| Some(clip.id) == self.view.selected)
        }) {
            return Vec::new();
        }
        let mut actions = vec![Action::ToggleAudioMute];
        for &parameter in recording_studio::audio::AudioParameter::ALL {
            for direction in [-1, 1] {
                actions.push(Action::AdjustAudio {
                    parameter,
                    direction,
                });
            }
        }
        actions
    }

    /// Audio clips wait for their real device clock; silent projects retain ordinary visual playback.
    pub(super) fn advance_playback(&mut self, elapsed: MediaTime, available: MediaTime) {
        let Some(editor) = &self.editor else {
            return;
        };
        let epoch = [self.document, editor.revision(), self.transport.epoch()];
        let audio = &self.view.audio;
        if editor.project().audio.is_empty() || audio.epoch == Some(epoch) && audio.error.is_some()
        {
            self.transport
                .advance_until(elapsed, editor.duration(), available);
        } else if audio.epoch == Some(epoch) && audio.engaged {
            self.transport
                .synchronize_audio(audio.position, editor.duration(), available);
        }
    }
}

impl crate::menu::MenuRuntime {
    /// Studio owns the audible device while any of its screens are visible, including the library.
    pub fn studio_audio_exclusive(&self) -> bool {
        self.visible && self.screen == launcher::menu::MenuScreen::RecordingStudio
    }

    /// The visual adapter's admitted frame bounds audio; metadata and seek epochs invalidate both workers.
    pub fn studio_audio_request(&self) -> Option<PreviewRequest> {
        let preview = self.studio_preview_request()?;
        if preview.project.audio.is_empty() || self.studio.view.workflow.show_settings {
            return None;
        }
        let ceiling = self
            .studio
            .view
            .preview
            .playback_ceiling(preview.epoch, preview.project.export.fps);
        Some(PreviewRequest {
            preview,
            duration: self.studio.editor.as_ref()?.duration(),
            playing: self.studio.transport.playing(),
            ceiling,
            speed: self.studio.view.workspace.monitoring.speed,
        })
    }

    /// Stale device progress cannot move another document; a newly reported audio failure pauses once.
    pub fn set_studio_audio_status(&mut self, status: PreviewStatus) {
        if let Some(epoch) = status.epoch {
            let Some(editor) = &self.studio.editor else {
                return;
            };
            if epoch
                != [
                    self.studio.document,
                    editor.revision(),
                    self.studio.transport.epoch(),
                ]
            {
                return;
            }
        }
        let reconciled = if let Some(editor) = &self.studio.editor {
            let available = status.epoch.and_then(|epoch| {
                self.studio
                    .view
                    .preview
                    .playback_ceiling(epoch, editor.project().export.fps)
            });
            self.studio.audio_pause.reconcile(
                &mut self.studio.transport,
                &status,
                editor.duration(),
                available,
            )
        } else {
            false
        };
        if self.studio.view.audio == status && !reconciled {
            return;
        }
        if status.error.is_some()
            && (status.epoch != self.studio.view.audio.epoch
                || status.error != self.studio.view.audio.error)
        {
            self.studio.transport.pause();
        }
        let view = Arc::make_mut(&mut self.studio.view);
        view.position = self.studio.transport.position();
        view.playing = self.studio.transport.playing();
        view.audio = status;
    }
}
