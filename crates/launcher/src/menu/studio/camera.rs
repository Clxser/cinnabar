//! Transient viewport state stays separate from the saved camera shots and their undo history.

use recording_studio::{
    Id, MediaTime,
    camera::{
        CameraDragAxis, CameraShot, CameraSnap, CameraSubjectFilter, CameraSubjectSearch,
        CameraTarget, CameraView, CompiledCameraPath,
    },
};
use std::sync::Arc;
mod subjects;
pub use subjects::{CAMERA_SUBJECT_ROWS, CameraSubjectResults, CameraSubjectsAction};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraWorkspace {
    pub tools: super::CameraTools,
    pub subject_filter: CameraSubjectFilter,
    pub subject_search: CameraSubjectSearch,
    pub subject_results: Option<CameraSubjectResults>,
    pub subject_first: usize,
    pub focused: bool,
    pub looking: bool,
    pub view: Option<CameraView>,
    pub key: Option<(Id, MediaTime)>,
    /// Pointer drags publish a replacement for overlay/preview only until one gesture commits.
    pub draft: Option<Arc<CameraShot>>,
    pub drag_axis: CameraDragAxis,
    pub snap: CameraSnap,
    pub guides: CameraGuides,
    pub saved_views: SavedCameraViews,
    pub path_cache: Option<CameraPathCache>,
}

impl CameraWorkspace {
    /// Browsing and direct selection share class admission and the exact same compiled search.
    pub fn subject_matches(&self, target: &CameraTarget, local_actor: u64) -> bool {
        self.subject_search
            .includes(self.subject_filter, target, local_actor)
    }
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

pub const SAVED_CAMERA_VIEWS_PER_PAGE: usize = 4;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedCameraViews {
    pub selected: Option<Id>,
    pub first: usize,
    pub draft: Option<CameraViewNameDraft>,
}

impl SavedCameraViews {
    /// Pages are bounded by the visible row policy shared by paint and keyboard focus.
    pub fn range(&self, total: usize) -> std::ops::Range<usize> {
        let start = self
            .first
            .min(total.saturating_sub(SAVED_CAMERA_VIEWS_PER_PAGE));
        start..start.saturating_add(SAVED_CAMERA_VIEWS_PER_PAGE).min(total)
    }

    /// Moving a page never changes the saved selection or the inspected camera.
    pub fn step(&mut self, total: usize, direction: i8) {
        let first = self.range(total).start;
        self.first = if direction < 0 {
            first.saturating_sub(SAVED_CAMERA_VIEWS_PER_PAGE)
        } else {
            first
                .saturating_add(SAVED_CAMERA_VIEWS_PER_PAGE)
                .min(total.saturating_sub(SAVED_CAMERA_VIEWS_PER_PAGE))
        };
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CameraViewNameDraft {
    pub id: Option<Id>,
    pub text: Arc<str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CameraViewsAction {
    Section(super::CameraTools),
    New,
    Select(Id),
    Recall(Id),
    Rename(Id),
    Delete(Id),
    Rebind(Id),
    Save,
    Cancel,
    Page(i8),
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
