//! Typed frame-owner handoff; the native menu never owns capture threads or the live world.

use super::*;
use recording_studio::capture::{CapturePhase, CaptureRequest, CaptureStatus};

impl Studio {
    /// Export freezes the current immutable project revision while further edits remain independent.
    pub(super) fn queue_export(&mut self) -> Result<(), String> {
        if self.export_request.is_some() || self.view.export.phase.busy() {
            return Ok(());
        }
        if self.view.workflow.export_range_draft.is_some() {
            return Err("Apply or cancel the Export In/Out field before exporting.".into());
        }
        let project = self
            .editor
            .as_ref()
            .ok_or("Open a project to export.")?
            .project();
        if project.duration() == MediaTime::ZERO {
            return Err("Add media before exporting.".into());
        }
        let request = recording_studio::export::ExportRequest {
            project: Arc::clone(project),
            source_base: self
                .view
                .file
                .as_ref()
                .and_then(|path| path.parent())
                .map_or_else(|| self.root.clone(), PathBuf::from),
            destination: match &self.view.workflow.destination {
                Some(path)
                    if path
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4")) =>
                {
                    recording_studio::export::Destination::File(path.clone())
                }
                Some(path) => recording_studio::export::Destination::Directory(path.clone()),
                None => recording_studio::export::Destination::Directory(self.root.join("exports")),
            },
        };
        request.frame_range()?;
        self.export_request = Some(request);
        self.transport.pause();
        self.export_cancel = false;
        Arc::make_mut(&mut self.view).export = recording_studio::export::ExportStatus {
            phase: recording_studio::export::ExportPhase::Preparing,
            ..Default::default()
        };
        Ok(())
    }

    /// Coalesces button repeats and never replaces a pending stop with a bookmark.
    pub(super) fn queue_capture_toggle(&mut self) {
        self.capture_request = match self.view.capture.phase {
            CapturePhase::Idle | CapturePhase::Complete | CapturePhase::Failed => {
                Some(CaptureRequest::Start(self.view.workflow.capture_settings))
            }
            CapturePhase::Armed | CapturePhase::Starting | CapturePhase::Recording => {
                Some(CaptureRequest::Stop)
            }
            CapturePhase::Finishing => None,
        };
    }

    /// Publishes phase and diagnostics immediately; frame counters follow the quantized capture clock.
    fn capture_status(&mut self, status: CaptureStatus) {
        let old = &self.view.capture;
        if old == &status {
            return;
        }
        let completed = (status.phase == CapturePhase::Complete
            || status.phase == CapturePhase::Failed && status.path.is_some())
            && self.view.capture.phase != status.phase;
        Arc::make_mut(&mut self.view).capture = status;
        if completed {
            if let Err(error) = self.refresh() {
                Arc::make_mut(&mut self.view).error = Some(error);
            }
        }
    }
}

impl crate::menu::MenuRuntime {
    /// Subject verification is supplied by the native scene owner and runs only on analysis workers.
    pub fn set_studio_target_coverage(
        &mut self,
        reader: recording_studio::camera::coverage::TargetCoverageReader,
    ) {
        self.studio.target_coverage = Some(reader);
    }

    /// Recovery runs through the existing storage worker after the app installs its scene owner.
    pub fn set_studio_recording_recovery(
        &mut self,
        recover: recording_studio::storage::RecordingRecovery,
    ) {
        self.studio.recording_recovery = Some(recover);
    }

    /// Only running transport admits speculative visual preparation; paused camera edits stay exact.
    pub fn studio_preview_playing(&self) -> bool {
        self.studio.transport.playing()
    }

    /// The image edge and PCM edge share the same runtime monitoring policy.
    pub fn studio_monitoring_speed(
        &self,
    ) -> recording_studio::playback::monitoring::MonitoringSpeed {
        self.studio.view.workspace.monitoring.speed
    }

    /// Export workers own their rendering state and receive one immutable project snapshot.
    pub fn take_studio_export_request(
        &mut self,
    ) -> Option<recording_studio::export::ExportRequest> {
        self.studio.export_request.take()
    }

    /// Button repeats coalesce into one cancellation intent, serviced even outside the editor.
    pub fn take_studio_export_cancel(&mut self) -> bool {
        std::mem::take(&mut self.studio.export_cancel)
    }

    /// Export completion never marks later metadata edits saved or replaces the active project.
    pub fn set_studio_export_status(&mut self, status: recording_studio::export::ExportStatus) {
        if self.studio.view.export != status {
            Arc::make_mut(&mut self.studio.view).export = status;
        }
    }

    /// The app prepares only the visible editor, retaining immutable project metadata across the handoff.
    pub fn studio_preview_request(&self) -> Option<recording_studio::preview::PreviewRequest> {
        if !self.visible
            || self.studio.view.export.phase.busy()
            || self.screen != launcher::menu::MenuScreen::RecordingStudio
            || self.studio.view.show_library
            || self.studio.view.workflow.show_settings
            || self.studio.view.workflow.recovery.visible
        {
            return None;
        }
        let editor = self.studio.editor.as_ref()?;
        if editor.duration() == MediaTime::ZERO {
            return None;
        }
        Some(recording_studio::preview::PreviewRequest {
            epoch: [
                self.studio.document,
                editor.revision(),
                self.studio.transport.epoch(),
            ],
            project: self
                .studio
                .camera_interaction
                .project
                .as_ref()
                .or(self.studio.graph.project.as_ref())
                .map_or_else(|| Arc::clone(editor.project()), Arc::clone),
            source_base: self
                .studio
                .view
                .file
                .as_ref()
                .and_then(|path| path.parent())
                .map_or_else(|| self.studio.root.clone(), PathBuf::from),
            at: self.studio.transport.position(),
            camera_view: self.studio.view.camera_workspace.view,
            quality: self.studio.view.workspace.preview_quality,
            cadence: None,
        })
    }

    /// Preview progress does not dirty the project or overwrite capture/editor errors.
    pub fn set_studio_preview_status(&mut self, status: recording_studio::preview::PreviewStatus) {
        if status.error.is_some() {
            self.studio.transport.pause();
        }
        if let Some(camera) = &status.camera {
            let view = Arc::make_mut(&mut self.studio.view);
            if view.camera_target.is_none() {
                view.camera_target = camera
                    .targets
                    .iter()
                    .filter(|target| {
                        view.camera_workspace
                            .subject_matches(target, camera.local_actor)
                    })
                    .min_by_key(|target| {
                        (target.runtime_id != camera.local_actor, target.runtime_id)
                    })
                    .map(recording_studio::camera::CameraTarget::reference);
            }
        }
        if self.studio.view.preview != status {
            Arc::make_mut(&mut self.studio.view).preview = status;
        }
        self.studio.reconcile_camera_subjects();
    }

    /// Allows the capture adapter to stay dormant without mutating resources during ordinary play.
    pub fn studio_capture_pending(&self) -> bool {
        self.studio.capture_request.is_some()
    }

    /// Transfers one native capture intent to the app's live-world adapter.
    pub fn take_studio_capture_request(&mut self) -> Option<CaptureRequest> {
        self.studio.capture_request.take()
    }

    /// Capture completion refreshes the library without replacing or discarding an edited project.
    pub fn set_studio_capture_status(&mut self, status: CaptureStatus) {
        self.studio.capture_status(status);
    }
}
