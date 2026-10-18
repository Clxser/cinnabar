//! Timeline workspace state is independent of saved project metadata and media contents.
use recording_studio::{Id, MediaTime, Span, timeline::TimelineKey, tracks::Track};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimelineAction {
    SelectAll,
    Duplicate,
    ToggleSnapping,
    ToggleLock(Track),
    ToggleMute(Track),
    DragKey {
        key: TimelineKey,
        left: i32,
        width: u32,
        height: u32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineWorkspace {
    pub selected: BTreeSet<Id>,
    pub anchor: Option<Id>,
    pub locked: BTreeSet<Track>,
    pub snapping: bool,
    pub snap_line: Option<MediaTime>,
    pub ghost: Vec<(Id, Span)>,
    pub key_drag: Option<(TimelineKey, MediaTime, i32)>,
}
impl Default for TimelineWorkspace {
    fn default() -> Self {
        Self {
            selected: BTreeSet::new(),
            anchor: None,
            locked: BTreeSet::new(),
            snapping: true,
            snap_line: None,
            ghost: Vec::new(),
            key_drag: None,
        }
    }
}
