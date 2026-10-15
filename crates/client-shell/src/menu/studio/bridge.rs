//! Typed frame-owner handoff; the native menu never owns capture threads or the live world.

use super::*;
use recording_studio::capture::{CapturePhase, CaptureRequest, CaptureStatus};

impl Studio {
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
