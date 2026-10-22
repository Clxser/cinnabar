//! Transient viewport state stays separate from the saved camera shots and their undo history.

use recording_studio::{
    Id, MediaTime,
    camera::{CameraShot, CameraSubjectFilter, CameraView},
};
use std::sync::Arc;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraWorkspace {
    pub subject_filter: CameraSubjectFilter,
    pub focused: bool,
    pub looking: bool,
    pub view: Option<CameraView>,
    pub key: Option<(Id, MediaTime)>,
    /// Pointer drags publish a replacement for overlay/preview only until one gesture commits.
    pub draft: Option<Arc<CameraShot>>,
}
