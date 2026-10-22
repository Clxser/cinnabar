//! Rhythm work stays off the UI thread; suggestions become ordinary undoable document edits.

use super::*;
use launcher::menu::studio::creative::{CreativeAction, audio_clip};
use recording_studio::{
    audio::beats,
    camera::{CameraPose, CameraPreset, CameraShot, CameraView},
    export::Cancellation,
    model::AudioClip,
};

#[derive(Default)]
pub(super) struct CreativeTasks {
    beat: Option<BeatJob>,
    analyzed: Option<BeatBasis>,
    shots: Option<ShotDraft>,
}

struct BeatBasis {
    clip: AudioClip,
    source: MediaSource,
}

impl BeatBasis {
    /// A tempo estimate belongs to the exact audible clip and source that were decoded.
    fn matches(&self, project: &Project) -> bool {
        project.audio.contains(&self.clip) && project.sources.contains(&self.source)
    }
}

struct BeatJob {
    document: u64,
    clip: AudioClip,
    source: MediaSource,
    cancellation: Cancellation,
    result: crossbeam_channel::Receiver<Result<beats::Estimate, String>>,
}

impl Drop for BeatJob {
    /// Replacing the project abandons decoding without waiting for a worker or FFmpeg on the UI thread.
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

struct ShotDraft {
    document: u64,
    project: Arc<Project>,
    shots: Arc<[CameraShot]>,
    framing: Vec<CameraPose>,
}

impl Studio {
    /// Explicit assists are independent of transport navigation and never overwrite existing camera shots.
    pub(super) fn creative_action(&mut self, action: CreativeAction) -> Result<(), String> {
        if self.creative.beat.is_some()
            && matches!(
                action,
                CreativeAction::StepTempo(_)
                    | CreativeAction::StepOffset(_)
                    | CreativeAction::BeatHere
                    | CreativeAction::ApplyBeatGrid
                    | CreativeAction::ClearBeatGrid
            )
        {
            return Err("Finish or cancel rhythm analysis before changing its tempo grid.".into());
        }
        match action {
            CreativeAction::DetectBeats => self.detect_beats()?,
            CreativeAction::CancelAnalysis => {
                self.creative.beat = None;
                let status = &mut Arc::make_mut(&mut self.view).workflow.creative;
                status.analyzing = false;
                status.message = Some("Beat analysis cancelled.".into());
            }
            CreativeAction::StepTempo(step) => {
                self.creative.analyzed = None;
                let status = &mut Arc::make_mut(&mut self.view).workflow.creative;
                status.tempo_tenths = (i32::from(status.tempo_tenths) + i32::from(step))
                    .clamp(i32::from(beats::MIN_TEMPO), i32::from(beats::MAX_TEMPO))
                    as u16;
                status.analyzed_clip = None;
                status.message = Some(
                    "Manual tempo grid. Listen and align its first beat before adding markers."
                        .into(),
                );
            }
            CreativeAction::StepOffset(millis) => {
                let status = &mut Arc::make_mut(&mut self.view).workflow.creative;
                let period =
                    60_000_i64 * i64::from(beats::TEMPO_SCALE) / i64::from(status.tempo_tenths);
                status.offset_millis =
                    (status.offset_millis + i64::from(millis)).rem_euclid(period.max(1));
            }
            CreativeAction::BeatHere => {
                let clip = self.beat_audio()?;
                let span = clip.span().map_err(|error| error.to_string())?;
                let at = self.transport.position();
                if !span.contains(at) {
                    return Err("Move the playhead inside the selected audio clip first.".into());
                }
                self.creative.analyzed = None;
                let status = &mut Arc::make_mut(&mut self.view).workflow.creative;
                let period =
                    60 * recording_studio::time::MICROS_PER_SECOND * u64::from(beats::TEMPO_SCALE)
                        / u64::from(status.tempo_tenths);
                status.offset_millis = (at.saturating_sub(span.start).0 % period / 1_000) as i64;
                status.analyzed_clip = None;
            }
            CreativeAction::ApplyBeatGrid => self.apply_beat_grid(false)?,
            CreativeAction::ClearBeatGrid => self.apply_beat_grid(true)?,
            CreativeAction::CycleLookStrength => {
                let status = &mut Arc::make_mut(&mut self.view).workflow.creative;
                status.look_strength = match status.look_strength {
                    25 => 50,
                    50 => 75,
                    75 => 100,
                    100 => 150,
                    _ => 25,
                };
            }
            CreativeAction::ToggleLookRange => {
                let status = &mut Arc::make_mut(&mut self.view).workflow.creative;
                status.look_selected_range = !status.look_selected_range;
            }
            CreativeAction::SuggestShots => self.suggest_camera_shots()?,
            CreativeAction::InspectShot(index) => self.inspect_suggested_shot(index)?,
            CreativeAction::ApplyShot(index) => self.apply_suggested_shot(index)?,
        }
        Ok(())
    }

    /// Resolves exactly the audio clip displayed by the assist panel, so the worker and UI agree.
    fn beat_audio(&self) -> Result<AudioClip, String> {
        audio_clip(
            self.project()?,
            self.view.selected,
            self.transport.position(),
        )
        .cloned()
        .ok_or_else(|| "Import music or select an audio clip before making beat markers.".into())
    }

    /// PCM decoding runs in one cancellable job and retains only a fixed-size rhythm accumulator.
    fn detect_beats(&mut self) -> Result<(), String> {
        if self.creative.beat.is_some() {
            return Err("Finish or cancel the current beat analysis first.".into());
        }
        let clip = self.beat_audio()?;
        let source = self
            .project()?
            .sources
            .iter()
            .find(|source| source.id == clip.source)
            .cloned()
            .ok_or("The selected audio source is missing.")?;
        let base = self
            .view
            .file
            .as_ref()
            .and_then(|path| path.parent())
            .unwrap_or(&self.root);
        let path = recording_studio::audio::source_path(&source, base);
        let worker_clip = clip.clone();
        let cancellation = Cancellation::default();
        let worker_cancel = cancellation.clone();
        let (completed, result) = crossbeam_channel::bounded(1);
        std::thread::Builder::new()
            .name("studio-beat-analysis".into())
            .spawn(move || {
                let result = beats::analyze(&path, &worker_clip, &worker_cancel);
                let _ = completed.send(result);
            })
            .map_err(|error| error.to_string())?;
        self.creative.beat = Some(BeatJob {
            document: self.document,
            clip,
            source,
            cancellation,
            result,
        });
        let status = &mut Arc::make_mut(&mut self.view).workflow.creative;
        status.analyzing = true;
        status.message = Some("Estimating rhythm in the background. You can keep editing.".into());
        Ok(())
    }

    /// A stale analysis cannot publish into another project or changed/retimed/relinked audio clip.
    pub(super) fn poll_creative(&mut self) {
        let result = match self.creative.beat.as_ref().map(|job| job.result.try_recv()) {
            Some(Ok(result)) => result,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                Err("The beat analysis worker stopped unexpectedly.".into())
            }
            _ => return,
        };
        let job = self.creative.beat.take().expect("analysis job exists");
        if job.document != self.document {
            return;
        }
        let unchanged = self.project().is_ok_and(|project| {
            project.audio.contains(&job.clip) && project.sources.contains(&job.source)
        });
        let status = &mut Arc::make_mut(&mut self.view).workflow.creative;
        status.analyzing = false;
        if !unchanged {
            status.message =
                Some("The audio changed during analysis. Analyze the new version again.".into());
            return;
        }
        match result {
            Ok(estimate) => {
                self.creative.analyzed = Some(BeatBasis {
                    clip: job.clip.clone(),
                    source: job.source.clone(),
                });
                status.tempo_tenths = estimate.bpm_tenths;
                status.offset_millis =
                    (estimate.first.saturating_sub(job.clip.at).0 / 1_000) as i64;
                status.analyzed_clip = Some(job.clip.id);
                status.message = Some(format!(
                    "Estimated {:.1} BPM from {} transients ({}% consistency). Listen and adjust before adding the grid.",
                    f64::from(estimate.bpm_tenths) / f64::from(beats::TEMPO_SCALE),
                    estimate.onsets,
                    (estimate.consistency * 100.0).round() as u8
                ));
            }
            Err(error) => status.message = Some(error),
        }
    }

    /// A whole tempo grid is one reversible transaction; other marker kinds are preserved.
    fn apply_beat_grid(&mut self, clear: bool) -> Result<(), String> {
        let clip = self.beat_audio()?;
        let span = clip.span().map_err(|error| error.to_string())?;
        let status = &self.view.workflow.creative;
        if !clear
            && self.creative.analyzed.as_ref().is_some_and(|basis| {
                self.project()
                    .map_or(true, |project| !basis.matches(project))
            })
        {
            return Err("The analyzed audio was retimed or relinked. Analyze it again or choose a manual tempo before adding markers.".into());
        }
        if !clear && status.analyzed_clip.is_some_and(|id| id != clip.id) {
            return Err("The estimate belongs to another audio clip. Analyze this clip or choose a manual tempo first.".into());
        }
        let markers = if clear {
            Vec::new()
        } else {
            beats::markers(
                self.project()?
                    .next_id()
                    .map_err(|error| error.to_string())?,
                span,
                status.tempo_tenths,
                status.offset_millis,
            )
            .map_err(|error| error.to_string())?
        };
        let count = markers.len();
        self.edit(
            if clear {
                "Clear beat grid"
            } else {
                "Add beat grid"
            },
            &[Command::SetBeatMarkers { span, markers }],
        )?;
        Arc::make_mut(&mut self.view).notice = Some(if clear {
            "Beat markers in this audio range were removed. Undo restores them.".into()
        } else {
            format!("Added {count} editable tempo markers. Undo removes the grid.")
        });
        Ok(())
    }

    /// Suggestions are alternatives for one clear native interval and retain the inspected framing.
    fn suggest_camera_shots(&mut self) -> Result<(), String> {
        let preview = self.camera_preview()?;
        if !preview.interactive {
            return Err("Choose one unframed 3D replay view before suggesting cameras.".into());
        }
        let project = Arc::clone(self.project()?);
        let at = self.transport.position();
        let mut end = MediaTime(at.0.saturating_add(4_000_000).min(project.duration().0));
        let mut native_end = None;
        for clip in project
            .clips
            .iter()
            .filter(|clip| clip.enabled && clip.source == preview.source)
        {
            let span = clip.span().map_err(|error| error.to_string())?;
            if span.contains(at) {
                native_end = Some(span.end);
                break;
            }
        }
        end =
            end.min(native_end.ok_or("Move the playhead onto the visible 3D replay clip first.")?);
        if project.shots.iter().any(|shot| shot.span.contains(at)) {
            return Err(
                "Move to an empty camera range first. Suggestions preserve your existing shots."
                    .into(),
            );
        }
        if let Some(next) = project
            .shots
            .iter()
            .filter(|shot| shot.span.start > at)
            .map(|shot| shot.span.start)
            .min()
        {
            end = end.min(next);
        }
        if end.saturating_sub(at).0 < 100_000 {
            return Err("Leave at least a tenth of a second of clear native replay for a camera suggestion.".into());
        }
        let target = self.view.camera_target.and_then(|reference| {
            preview
                .targets
                .iter()
                .find(|target| target.matches(reference))
        });
        let pose = self
            .view
            .camera_workspace
            .view
            .map_or(preview.pose, |view| view.pose);
        let span = Span::new(at, end).map_err(|error| error.to_string())?;
        let first = project.next_id().map_err(|error| error.to_string())?;
        let mut shots = Vec::new();
        let mut framing = Vec::new();
        for preset in CameraPreset::ALL
            .iter()
            .copied()
            .filter(|preset| !preset.needs_subject() || target.is_some())
        {
            let id = Id(first
                .0
                .checked_add(shots.len() as u64)
                .ok_or("Camera identifiers are exhausted.")?);
            if let Ok(shot) = CameraShot::cinematic_preset(id, span, preset, pose, target)
                && let Some(pose) = shot.sample(at, |reference| {
                    preview
                        .targets
                        .iter()
                        .find(|target| target.matches(reference))
                        .map(|target| target.pose)
                })
            {
                framing.push(pose);
                shots.push(shot);
            }
        }
        if shots.is_empty() {
            return Err(
                "No camera suggestions fit this view. Move farther from the subject and try again."
                    .into(),
            );
        }
        let shots: Arc<[CameraShot]> = shots.into();
        self.creative.shots = Some(ShotDraft {
            document: self.document,
            project,
            shots: Arc::clone(&shots),
            framing,
        });
        Arc::make_mut(&mut self.view).workflow.creative.shots = shots;
        Ok(())
    }

    /// Changed documents invalidate alternatives rather than overwriting later authored work.
    fn suggested_shot(&self, index: usize) -> Result<CameraShot, String> {
        let draft = self
            .creative
            .shots
            .as_ref()
            .ok_or("Generate camera suggestions first.")?;
        if draft.document != self.document || !Arc::ptr_eq(self.project()?, &draft.project) {
            return Err(
                "The timeline changed. Generate fresh suggestions for the current view.".into(),
            );
        }
        draft
            .shots
            .get(index)
            .cloned()
            .ok_or_else(|| "This camera suggestion is no longer available.".into())
    }

    /// Inspecting a proposal changes only the transient view; adding it remains an explicit document edit.
    fn inspect_suggested_shot(&mut self, index: usize) -> Result<(), String> {
        let shot = self.suggested_shot(index)?;
        let pose = *self
            .creative
            .shots
            .as_ref()
            .and_then(|draft| draft.framing.get(index))
            .ok_or("The suggested camera's recorded framing is unavailable.")?;
        self.transport.pause();
        let duration = self.project()?.duration();
        self.transport.seek(shot.span.start, duration);
        self.release_camera();
        let workspace = &mut Arc::make_mut(&mut self.view).camera_workspace;
        let revision = workspace
            .view
            .map_or(1, |view| view.revision.wrapping_add(1));
        workspace.view = Some(CameraView { pose, revision });
        Ok(())
    }

    /// Adds one chosen alternative as a normal camera shot with normal undo, keyframes and export behavior.
    fn apply_suggested_shot(&mut self, index: usize) -> Result<(), String> {
        let shot = self.suggested_shot(index)?;
        let id = shot.id;
        self.edit("Add suggested camera", &[Command::AddShot { shot }])?;
        self.reset_camera_view();
        self.creative.shots = None;
        let view = Arc::make_mut(&mut self.view);
        view.workflow.creative.shots = Arc::default();
        view.selected = Some(id);
        view.workspace.inspector_hidden = false;
        Ok(())
    }

    /// A document edit retires old camera alternatives before the next presentation is published.
    pub(super) fn reconcile_creative(&mut self) {
        if self.creative.analyzed.as_ref().is_some_and(|basis| {
            self.project()
                .map_or(true, |project| !basis.matches(project))
        }) {
            Arc::make_mut(&mut self.view).workflow.creative.message = Some(
                "Audio changed after rhythm analysis. Analyze again or choose a manual tempo."
                    .into(),
            );
        }
        let stale = self.creative.shots.as_ref().is_some_and(|draft| {
            draft.document != self.document
                || self
                    .project()
                    .map_or(true, |project| !Arc::ptr_eq(project, &draft.project))
        });
        if stale {
            self.creative.shots = None;
            Arc::make_mut(&mut self.view).workflow.creative.shots = Arc::default();
        }
    }

    /// Keyboard fallback enumerates the same assist controls as painted hit targets.
    pub(super) fn creative_focus_actions(&self) -> Vec<Action> {
        let mut actions = vec![
            CreativeAction::CycleLookStrength,
            CreativeAction::ToggleLookRange,
            CreativeAction::SuggestShots,
        ];
        if self.creative.beat.is_some() {
            actions.push(CreativeAction::CancelAnalysis);
        } else if self.project().is_ok_and(|project| {
            audio_clip(project, self.view.selected, self.transport.position()).is_some()
        }) {
            actions.extend([
                CreativeAction::DetectBeats,
                CreativeAction::StepTempo(-10),
                CreativeAction::StepTempo(-1),
                CreativeAction::StepTempo(1),
                CreativeAction::StepTempo(10),
                CreativeAction::StepOffset(-10),
                CreativeAction::StepOffset(10),
                CreativeAction::BeatHere,
                CreativeAction::ApplyBeatGrid,
                CreativeAction::ClearBeatGrid,
            ]);
        }
        for index in 0..self.view.workflow.creative.shots.len() {
            actions.extend([
                CreativeAction::InspectShot(index),
                CreativeAction::ApplyShot(index),
            ]);
        }
        actions.into_iter().map(Action::Creative).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recording_studio::{PlaybackRate, model::AudioRole};

    /// Metadata-only music makes worker publication testable without FFmpeg, files or a device clock.
    fn music_project() -> Project {
        let mut project = Project::new("Music edit");
        project.sources.push(MediaSource {
            id: Id(1),
            name: "Music".into(),
            path: "music.wav".into(),
            kind: MediaKind::Audio,
            duration: MediaTime(10_000_000),
            size: [0, 0],
            frame_rate: None,
            sha256: None,
        });
        project.audio.push(AudioClip {
            id: Id(2),
            source: Id(1),
            linked_clip: None,
            source_range: Span::new(MediaTime::ZERO, MediaTime(10_000_000)).unwrap(),
            at: MediaTime::ZERO,
            rate: PlaybackRate::default(),
            speed_ramp: None,
            role: AudioRole::Music,
            gain: 1.0,
            fade_in: MediaTime::ZERO,
            fade_out: MediaTime::ZERO,
            muted: false,
        });
        project
    }

    #[test]
    fn a_late_rhythm_estimate_cannot_overwrite_a_retimed_audio_clips_settings() {
        let project = music_project();
        let mut studio = Studio::new(PathBuf::new());
        studio.install(Editor::new(project.clone()).unwrap(), None);
        let (completed, result) = crossbeam_channel::bounded(1);
        studio.creative.beat = Some(BeatJob {
            document: studio.document,
            clip: project.audio[0].clone(),
            source: project.sources[0].clone(),
            cancellation: Default::default(),
            result,
        });
        Arc::make_mut(&mut studio.view).workflow.creative.analyzing = true;
        let mut clip = project.audio[0].clone();
        clip.rate = PlaybackRate {
            numerator: 2,
            denominator: 1,
        };
        studio
            .edit("Retime music", &[Command::ReplaceAudio { audio: clip }])
            .unwrap();
        completed
            .send(Ok(beats::Estimate {
                bpm_tenths: 1300,
                first: MediaTime(125_000),
                onsets: 20,
                consistency: 0.9,
            }))
            .unwrap();
        studio.poll_creative();
        let status = &studio.view.workflow.creative;
        assert!(!status.analyzing);
        assert_eq!(status.tempo_tenths, 120 * beats::TEMPO_SCALE);
        assert!(
            status
                .message
                .as_ref()
                .unwrap()
                .contains("changed during analysis")
        );
    }

    #[test]
    fn replacing_the_document_cancels_analysis_and_removes_the_previous_proposals() {
        let project = music_project();
        let mut studio = Studio::new(PathBuf::new());
        studio.install(Editor::new(project.clone()).unwrap(), None);
        let cancellation = Cancellation::default();
        let (_completed, result) = crossbeam_channel::bounded(1);
        studio.creative.beat = Some(BeatJob {
            document: studio.document,
            clip: project.audio[0].clone(),
            source: project.sources[0].clone(),
            cancellation: cancellation.clone(),
            result,
        });
        Arc::make_mut(&mut studio.view).workflow.creative.analyzing = true;
        studio.install(Editor::new(Project::new("Another project")).unwrap(), None);
        assert!(cancellation.cancelled());
        assert!(studio.creative.beat.is_none());
        assert!(!studio.view.workflow.creative.analyzing);
        assert!(studio.view.workflow.creative.shots.is_empty());
    }

    #[test]
    fn completed_rhythm_estimates_require_reanalysis_after_retime_or_relink() {
        for relink in [false, true] {
            let project = music_project();
            let mut studio = Studio::new(PathBuf::new());
            studio.install(Editor::new(project.clone()).unwrap(), None);
            let (completed, result) = crossbeam_channel::bounded(1);
            studio.creative.beat = Some(BeatJob {
                document: studio.document,
                clip: project.audio[0].clone(),
                source: project.sources[0].clone(),
                cancellation: Default::default(),
                result,
            });
            completed
                .send(Ok(beats::Estimate {
                    bpm_tenths: 1300,
                    first: MediaTime(125_000),
                    onsets: 20,
                    consistency: 0.9,
                }))
                .unwrap();
            studio.poll_creative();
            assert_eq!(studio.view.workflow.creative.analyzed_clip, Some(Id(2)));
            if relink {
                let mut source = project.sources[0].clone();
                source.path = "another-song.wav".into();
                studio
                    .edit("Relink music", &[Command::ReplaceSource { source }])
                    .unwrap();
            } else {
                let mut audio = project.audio[0].clone();
                audio.rate = PlaybackRate {
                    numerator: 2,
                    denominator: 1,
                };
                studio
                    .edit("Retime music", &[Command::ReplaceAudio { audio }])
                    .unwrap();
            }
            assert!(
                studio
                    .apply_beat_grid(false)
                    .unwrap_err()
                    .contains("retimed or relinked")
            );
            assert!(studio.project().unwrap().markers.is_empty());
            studio
                .creative_action(CreativeAction::StepTempo(0))
                .unwrap();
            studio.apply_beat_grid(false).unwrap();
            assert!(!studio.project().unwrap().markers.is_empty());
        }
    }
}
