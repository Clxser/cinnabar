//! Typed frame-owner handoff; the native menu never owns capture threads or the live world.

use super::*;
use recording_studio::capture::{CapturePhase, CaptureRequest, CaptureStatus};

impl Studio {
    /// Export freezes the current immutable project revision while further edits remain independent.
    fn queue_export(&mut self) -> Result<(), String> {
        if self.export_request.is_some() || self.view.export.phase.busy() {
            return Ok(());
        }
        let project = self
            .editor
            .as_ref()
            .ok_or("Open a project to export.")?
            .project();
        if project.duration() == MediaTime::ZERO {
            return Err("Add media before exporting.".into());
        }
        self.export_request = Some(recording_studio::export::ExportRequest {
            project: Arc::clone(project),
            source_base: self
                .view
                .file
                .as_ref()
                .and_then(|path| path.parent())
                .map_or_else(|| self.root.clone(), PathBuf::from),
            destination: recording_studio::export::Destination::Directory(
                self.root.join("exports"),
            ),
        });
        self.transport.pause();
        self.export_cancel = false;
        Arc::make_mut(&mut self.view).export = recording_studio::export::ExportStatus {
            phase: recording_studio::export::ExportPhase::Preparing,
            ..Default::default()
        };
        Ok(())
    }

    /// Coalesces button repeats and never replaces a pending stop with a bookmark.
    fn queue_capture_toggle(&mut self) {
        self.capture_request = match self.view.capture.phase {
            CapturePhase::Idle | CapturePhase::Complete | CapturePhase::Failed => {
                Some(CaptureRequest::Start)
            }
            CapturePhase::Armed | CapturePhase::Starting | CapturePhase::Recording => {
                Some(CaptureRequest::Stop)
            }
            CapturePhase::Finishing => None,
        };
    }

    /// Publishes compact progress only when it changed, preserving immutable menu snapshots.
    fn capture_status(&mut self, status: CaptureStatus) {
        if self.view.capture == status {
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
        {
            return None;
        }
        let editor = self.studio.editor.as_ref()?;
        if editor.project().sources.is_empty() || editor.project().duration() == MediaTime::ZERO {
            return None;
        }
        Some(recording_studio::preview::PreviewRequest {
            epoch: [
                self.studio.document,
                editor.revision(),
                self.studio.transport.epoch(),
            ],
            project: Arc::clone(editor.project()),
            source_base: self
                .studio
                .view
                .file
                .as_ref()
                .and_then(|path| path.parent())
                .map_or_else(|| self.studio.root.clone(), PathBuf::from),
            at: self.studio.transport.position(),
        })
    }

    /// Preview progress does not dirty the project or overwrite capture/editor errors.
    pub fn set_studio_preview_status(&mut self, status: recording_studio::preview::PreviewStatus) {
        if let Some(camera) = &status.camera {
            let view = Arc::make_mut(&mut self.studio.view);
            if view.camera_target.is_none() {
                view.camera_target = camera
                    .targets
                    .iter()
                    .find(|target| target.runtime_id == camera.local_actor)
                    .or_else(|| camera.targets.first())
                    .map(recording_studio::camera::CameraTarget::reference);
            }
        }
        if self.studio.view.preview != status {
            Arc::make_mut(&mut self.studio.view).preview = status;
        }
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
