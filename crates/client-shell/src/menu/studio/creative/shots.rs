//! Subject-bound alternatives are offered only after asynchronous source coverage verification.

use super::*;
use recording_studio::camera::coverage::{self, TargetCoverageRequest};
use recording_studio::retime::TimeMap;

const MIN_SUGGESTED_SHOT: MediaTime = MediaTime(100_000);

pub(super) struct ShotJob {
    draft: ShotDraft,
    epoch: u64,
    position: MediaTime,
    camera_view: Option<CameraView>,
    selected_target: Option<recording_studio::camera::CameraTargetRef>,
    map: TimeMap,
    source_duration: MediaTime,
    cancellation: Cancellation,
    result: crossbeam_channel::Receiver<coverage::TargetCoverageResult>,
}

impl Studio {
    /// One replaceable analysis is cancelled atomically; no stale alternatives remain actionable.
    pub(super) fn cancel_shot_analysis(&mut self) {
        if self.creative.shot_analysis.take().is_some() {
            let status = &mut Arc::make_mut(&mut self.view).workflow.creative;
            status.shot_analyzing = false;
            status.shot_message = Some("Camera verification cancelled.".into());
        }
    }

    /// A seek, source edit, target change or navigation change invalidates pending verification immediately.
    pub(super) fn reconcile_shot_coverage(&mut self) {
        if self
            .creative
            .shot_analysis
            .as_ref()
            .is_some_and(|job| !job.matches(self))
        {
            self.cancel_shot_analysis();
            Arc::make_mut(&mut self.view).workflow.creative.shot_message = Some("The source, playhead or inspected view changed. Generate fresh camera suggestions.".into());
        }
    }

    /// Only a matching completed source proof may publish shortened subject-bound alternatives.
    pub(super) fn poll_shot_coverage(&mut self) {
        self.reconcile_shot_coverage();
        let result = match self
            .creative
            .shot_analysis
            .as_ref()
            .map(|job| job.result.try_recv())
        {
            Some(Ok(result)) => result,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                Err("Camera verification stopped unexpectedly.".into())
            }
            _ => return,
        };
        let mut job = self
            .creative
            .shot_analysis
            .take()
            .expect("camera analysis completed");
        Arc::make_mut(&mut self.view)
            .workflow
            .creative
            .shot_analyzing = false;
        let coverage = match result {
            Ok(coverage) => coverage,
            Err(error) => {
                Arc::make_mut(&mut self.view).workflow.creative.shot_message = Some(error);
                return;
            }
        };
        let mut retained = Vec::new();
        let mut framing = Vec::new();
        let mut shortened = 0;
        let mut rejected = 0;
        for (shot, pose) in job.draft.shots.iter().zip(&job.draft.framing) {
            if shot.target_reference().is_none() {
                retained.push(shot.clone());
                framing.push(*pose);
                continue;
            }
            let Some(coverage) = coverage else {
                rejected += 1;
                continue;
            };
            let reference = shot.target_reference().expect("subject-bound shot");
            if reference.runtime_id != coverage.target.runtime_id
                || reference
                    .lifetime
                    .is_some_and(|lifetime| Some(lifetime) != coverage.target.lifetime)
            {
                rejected += 1;
                continue;
            }
            let span =
                coverage::timeline_prefix(&job.map, shot.span, job.source_duration, coverage);
            let Ok(Some(span)) = span else {
                rejected += 1;
                continue;
            };
            if span.duration() < MIN_SUGGESTED_SHOT {
                rejected += 1;
                continue;
            }
            let was_shortened = span.end < shot.span.end;
            let Ok(mut shot) = shot.trimmed(span) else {
                rejected += 1;
                continue;
            };
            shortened += usize::from(was_shortened);
            shot.target_lifetime = coverage.target.lifetime;
            retained.push(shot);
            framing.push(*pose);
        }
        job.draft.shots = retained.into();
        job.draft.framing = framing;
        job.draft.verified_target = coverage;
        let message = Some(format!(
            "Subject coverage verified. {shortened} alternatives shortened to the recorded lifetime; {rejected} unavailable alternatives omitted."
        ));
        let draft = ShotDraft {
            document: job.draft.document,
            project: Arc::clone(&job.draft.project),
            shots: Arc::clone(&job.draft.shots),
            framing: std::mem::take(&mut job.draft.framing),
            verified_target: job.draft.verified_target,
        };
        self.publish_shot_draft(draft, message);
    }

    /// Published alternatives and their proof become visible as one immutable editor snapshot.
    fn publish_shot_draft(&mut self, draft: ShotDraft, message: Option<String>) {
        let shots = Arc::clone(&draft.shots);
        self.creative.shots = Some(draft);
        let status = &mut Arc::make_mut(&mut self.view).workflow.creative;
        status.shot_analyzing = false;
        status.shot_message = message;
        status.shots = shots;
    }
}

impl Drop for ShotJob {
    /// Cancelling or replacing an assist never waits for archive IO on the UI thread.
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[cfg(test)]
mod tests;

impl ShotJob {
    /// Results belong to the precise document, source retiming, seek and inspected framing that requested them.
    fn matches(&self, studio: &Studio) -> bool {
        self.draft.document == studio.document
            && studio
                .project()
                .is_ok_and(|project| Arc::ptr_eq(project, &self.draft.project))
            && self.epoch == studio.transport.epoch()
            && self.position == studio.transport.position()
            && self.camera_view == studio.view.camera_workspace.view
            && self.selected_target == studio.view.camera_target
    }
}

impl Studio {
    /// Suggestions are alternatives for one clear native interval and retain the inspected framing.
    pub(super) fn suggest_camera_shots(&mut self) -> Result<(), String> {
        self.cancel_shot_analysis();
        let preview = self.camera_preview()?;
        if !preview.interactive {
            return Err("Choose one unframed 3D replay view before suggesting cameras.".into());
        }
        let project = Arc::clone(self.project()?);
        let at = self.transport.position();
        let mut end = MediaTime(at.0.saturating_add(4_000_000).min(project.duration().0));
        let mut native_clip = None;
        for clip in project.clips.iter().filter(|clip| {
            clip.enabled
                && clip.source == preview.source
                && !project
                    .muted_tracks
                    .contains(&recording_studio::tracks::Track::Video(clip.track))
        }) {
            let span = clip.span().map_err(|error| error.to_string())?;
            if span.contains(at) {
                if native_clip.is_some() {
                    return Err(
                        "Choose one unambiguous native clip before suggesting subject cameras."
                            .into(),
                    );
                }
                native_clip = Some(clip.clone());
            }
        }
        let clip = native_clip.ok_or("Move the playhead onto the visible 3D replay clip first.")?;
        end = end.min(clip.span().map_err(|error| error.to_string())?.end);
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
        if end.saturating_sub(at) < MIN_SUGGESTED_SHOT {
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
        let draft = ShotDraft {
            document: self.document,
            project: Arc::clone(&project),
            shots: Arc::clone(&shots),
            framing,
            verified_target: None,
        };
        self.creative.shots = None;
        Arc::make_mut(&mut self.view).workflow.creative.shots = Arc::default();
        if !shots.iter().any(|shot| shot.target_reference().is_some()) {
            self.publish_shot_draft(draft, None);
            return Ok(());
        }
        let reader = self.target_coverage.ok_or("Native subject verification is unavailable. Reopen Studio after the scene service starts.")?;
        let source = project
            .sources
            .iter()
            .find(|source| source.id == clip.source)
            .ok_or("The native replay source is missing.")?;
        let base = self
            .view
            .file
            .as_ref()
            .and_then(|path| path.parent())
            .unwrap_or(&self.root);
        let directory = if source.path.is_absolute() {
            source.path.clone()
        } else {
            base.join(&source.path)
        };
        let map = clip.time_map().map_err(|error| error.to_string())?;
        let query = TargetCoverageRequest {
            directory,
            source_range: coverage::source_window(&map, span, source.duration)
                .map_err(|error| error.to_string())?,
            target: target
                .expect("subject-bound candidate admitted")
                .reference(),
            require_eye: false,
        };
        self.transport.pause();
        let cancellation = Cancellation::default();
        if self.target_verifier.is_none() {
            self.target_verifier = Some(coverage::TargetVerifier::start()?);
        }
        let result = self
            .target_verifier
            .as_ref()
            .expect("verifier started")
            .submit(reader, query, cancellation.clone())?;
        self.creative.shot_analysis = Some(ShotJob {
            draft,
            epoch: self.transport.epoch(),
            position: self.transport.position(),
            camera_view: self.view.camera_workspace.view,
            selected_target: self.view.camera_target,
            map,
            source_duration: source.duration,
            cancellation,
            result,
        });
        let status = &mut Arc::make_mut(&mut self.view).workflow.creative;
        status.shot_analyzing = true;
        status.shot_message =
            Some("Verifying the recorded subject through the whole proposed shot…".into());
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
        let shot = draft
            .shots
            .get(index)
            .cloned()
            .ok_or_else(|| "This camera suggestion is no longer available.".to_string())?;
        if let Some(reference) = shot.target_reference()
            && draft
                .verified_target
                .is_none_or(|coverage| coverage.target != reference)
        {
            return Err("This subject-bound shot has no verified lifetime coverage. Generate fresh suggestions.".into());
        }
        Ok(shot)
    }

    /// Inspecting a proposal changes only the transient view; adding it remains an explicit document edit.
    pub(super) fn inspect_suggested_shot(&mut self, index: usize) -> Result<(), String> {
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
    pub(super) fn apply_suggested_shot(&mut self, index: usize) -> Result<(), String> {
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
}
