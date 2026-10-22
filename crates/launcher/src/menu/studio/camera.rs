//! Transient viewport state stays separate from the saved camera shots and their undo history.

use recording_studio::{
    Id, MediaTime,
    camera::{CameraDragAxis, CameraPose, CameraShot, CameraSnap, CameraSubjectFilter, CameraView},
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
    pub drag_axis: CameraDragAxis,
    pub snap: CameraSnap,
    pub guides: CameraGuides,
    /// Session bookmarks are inspection views, distinct from authored and saved timeline shots.
    pub bookmarks: Vec<CameraBookmark>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraBookmark {
    pub source: Id,
    pub pose: CameraPose,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CameraGuides {
    #[default]
    Off,
    Thirds,
    Safe,
}

impl CameraGuides {
    /// Composition guides affect only editor overlays, never exported media.
    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::Thirds,
            Self::Thirds => Self::Safe,
            Self::Safe => Self::Off,
        }
    }

    /// Short overlay names fit the camera tool's compact selector.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Thirds => "Thirds",
            Self::Safe => "Safe area",
        }
    }
}
