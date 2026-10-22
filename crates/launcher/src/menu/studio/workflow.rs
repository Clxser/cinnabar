//! Immutable recording, media and export control state.

use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workflow {
    pub recovery: Recovery,
    pub diagnostics: Diagnostics,
    pub show_settings: bool,
    pub capture_settings: recording_studio::capture::CaptureSettings,
    pub media_busy: bool,
    pub media_label: Option<String>,
    pub destination: Option<PathBuf>,
    pub destination_draft: Option<Arc<str>>,
    pub export_range_draft: Option<(super::ExportRangeField, Arc<str>)>,
    pub thumbnail_sheet: Option<Arc<recording_studio::media::ThumbnailSheet>>,
    pub thumbnail_slots: Arc<[Option<[u16; 4]>]>,
    pub thumbnail_icon: Option<ui::IconRef>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recovery {
    pub visible: bool,
    pub entries: Arc<[recording_studio::storage::RecoveryEntry]>,
    pub loading: bool,
    pub busy: bool,
    pub autosaving: bool,
    pub last_revision: Option<u64>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diagnostics {
    pub checking: bool,
    pub report: Option<Arc<recording_studio::video::diagnostics::Report>>,
    pub error: Option<String>,
}
