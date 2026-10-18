//! Runtime-only viewer controls remain distinct from saved clip retiming and export ranges.

use recording_studio::playback::monitoring::MonitoringSpeed;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MonitoringAction {
    Speed(MonitoringSpeed),
    StepSpeed(i8),
    Jump(i64),
    Start,
    End,
    ToggleLoop,
    EditSeek,
    ApplySeek,
    CancelSeek,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MonitoringWorkspace {
    pub speed: MonitoringSpeed,
    pub looping: bool,
    pub seek_draft: Option<Arc<str>>,
}
