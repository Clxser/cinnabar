//! Immutable recording, media and export control state.

use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Workflow {
    pub show_settings: bool,
    pub capture_settings: recording_studio::capture::CaptureSettings,
    pub media_busy: bool,
    pub media_label: Option<String>,
    pub destination: Option<PathBuf>,
    pub destination_draft: Option<Arc<str>>,
    pub thumbnail_sheet: Option<Arc<recording_studio::media::ThumbnailSheet>>,
    pub thumbnail_slots: Arc<[Option<[u16; 4]>]>,
    pub thumbnail_icon: Option<ui::IconRef>,
}
