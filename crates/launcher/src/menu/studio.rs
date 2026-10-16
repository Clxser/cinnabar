//! Native creator workspace actions and immutable presentation state.

use std::{path::PathBuf, sync::Arc};

use recording_studio::{Id, MediaTime, PlaybackRate, Project, storage::LibraryEntry};

pub const TITLE: &str = "Recording Studio";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    ToggleRecording,
    Bookmark,
    NewProject,
    Refresh,
    Open(usize),
    Library,
    Save,
    Undo,
    Redo,
    TogglePlayback,
    Step(i8),
    Seek(MediaTime),
    /// Logical-pixel ruler geometry; pointer input resolves this into a frame-snapped seek.
    Scrub {
        left: i32,
        width: u32,
    },
    Select(Id),
    Split,
    DeleteSelected,
    SetRate(PlaybackRate),
    AddTitle,
    UseNativeReplay,
    Zoom(i8),
    Pan(i8),
    ConfirmDiscard,
    CancelDiscard,
    CancelOpen,
    ClearError,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscardIntent {
    NewProject,
    Open(usize),
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub capture: recording_studio::capture::CaptureStatus,
    pub library: Arc<[LibraryEntry]>,
    pub library_loading: bool,
    pub library_truncated: bool,
    pub show_library: bool,
    pub project: Option<Arc<Project>>,
    pub file: Option<PathBuf>,
    pub position: MediaTime,
    pub playing: bool,
    pub dirty: bool,
    pub undo: Option<String>,
    pub redo: Option<String>,
    pub selected: Option<Id>,
    pub visible_start: MediaTime,
    pub visible_duration: MediaTime,
    pub saving: bool,
    pub loading: bool,
    pub discard: Option<DiscardIntent>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub preview: recording_studio::preview::PreviewStatus,
    pub preview_icon: Option<ui::IconRef>,
}

impl PartialEq for Snapshot {
    /// Project and library payloads are immutable publications; equality never traverses the timeline.
    fn eq(&self, other: &Self) -> bool {
        let Self {
            capture,
            library,
            library_loading,
            library_truncated,
            show_library,
            project,
            file,
            position,
            playing,
            dirty,
            undo,
            redo,
            selected,
            visible_start,
            visible_duration,
            saving,
            loading,
            discard,
            error,
            notice,
            preview,
            preview_icon,
        } = self;
        let Self {
            capture: b_capture,
            library: b_library,
            library_loading: b_library_loading,
            library_truncated: b_library_truncated,
            show_library: b_show_library,
            project: b_project,
            file: b_file,
            position: b_position,
            playing: b_playing,
            dirty: b_dirty,
            undo: b_undo,
            redo: b_redo,
            selected: b_selected,
            visible_start: b_visible_start,
            visible_duration: b_visible_duration,
            saving: b_saving,
            loading: b_loading,
            discard: b_discard,
            error: b_error,
            notice: b_notice,
            preview: b_preview,
            preview_icon: b_preview_icon,
        } = other;
        capture == b_capture
            && preview == b_preview
            && preview_icon == b_preview_icon
            && (Arc::ptr_eq(library, b_library) || library.is_empty() && b_library.is_empty())
            && match (project, b_project) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
            && (
                library_loading,
                library_truncated,
                show_library,
                file,
                position,
                playing,
                dirty,
                undo,
                redo,
            ) == (
                b_library_loading,
                b_library_truncated,
                b_show_library,
                b_file,
                b_position,
                b_playing,
                b_dirty,
                b_undo,
                b_redo,
            )
            && (
                selected,
                visible_start,
                visible_duration,
                saving,
                loading,
                discard,
                error,
                notice,
            ) == (
                b_selected,
                b_visible_start,
                b_visible_duration,
                b_saving,
                b_loading,
                b_discard,
                b_error,
                b_notice,
            )
    }
}
