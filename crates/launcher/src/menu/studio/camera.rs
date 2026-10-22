//! Transient viewport state stays separate from the saved camera shots and their undo history.

use recording_studio::{
    Id, MediaTime,
    camera::{
        CameraDragAxis, CameraPose, CameraShot, CameraSnap, CameraSubjectFilter, CameraView,
        CompiledCameraPath,
    },
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
    pub path_cache: Option<CameraPathCache>,
}

#[derive(Clone)]
pub struct CameraPathCache {
    pub shot: Id,
    pub epoch: [u64; 2],
    pub draft: Option<Arc<CameraShot>>,
    pub path: Arc<CompiledCameraPath>,
}

impl CameraPathCache {
    /// Document revisions and immutable draft identities invalidate a rail once per metadata edit.
    pub fn matches(&self, shot: Id, epoch: [u64; 2], draft: Option<&Arc<CameraShot>>) -> bool {
        self.shot == shot
            && self.epoch == epoch
            && match (self.draft.as_ref(), draft) {
                (None, None) => true,
                (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                _ => false,
            }
    }
}

impl PartialEq for CameraPathCache {
    /// UI equality compares cache identities without traversing every arc table on ordinary frames.
    fn eq(&self, other: &Self) -> bool {
        self.matches(other.shot, other.epoch, other.draft.as_ref())
            && Arc::ptr_eq(&self.path, &other.path)
    }
}

impl std::fmt::Debug for CameraPathCache {
    /// Debug output describes publication identity without dumping recorded geometry.
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output
            .debug_struct("CameraPathCache")
            .field("shot", &self.shot)
            .field("epoch", &self.epoch)
            .finish_non_exhaustive()
    }
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
