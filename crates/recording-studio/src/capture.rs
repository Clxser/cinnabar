//! Native live-capture contracts and point-of-view video packages.
//!
//! POV video includes camera/event metadata but is not a reconstructed 3D world replay.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    FrameRate, MediaTime,
    model::{Encoder, ExportCodec},
};

mod manifest;
mod playback;
mod recovery;
mod session;
mod size;
pub use manifest::{CaptureManifest, CaptureTrack};
pub use playback::{CameraRecording, RecordedView};
pub use recovery::recover_project;
pub use session::{CaptureSession, CaptureSummary};
pub use size::CaptureSize;

/// Versioned camera checkpoints bound seeks without replaying every camera sample in a long take.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PovCheckpoint {
    version: u32,
    settings: CaptureSettings,
    camera: Option<crate::camera::CameraPose>,
}
const POV_CHECKPOINT_VERSION: u32 = 1;

pub const POV_SCENE_SCHEMA: &str = "cinnabar.pov-video/1";
pub const VIDEO_FILE: &str = "video.mp4";

/// A captured POV file may have native scene tracks beside it; playback must admit them separately.
pub fn native_replay_directory(source: &crate::model::MediaSource) -> Option<&std::path::Path> {
    if source.kind == crate::model::MediaKind::Video && source.path.file_name()? == VIDEO_FILE {
        source.path.parent()
    } else {
        None
    }
}
pub const AUDIO_FILE: &str = "game.wav";
const MANIFEST_FILE: &str = "capture.json";
const MANIFEST_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_CAPTURE_HIGHLIGHTS: usize = 16_384;
pub const MAX_CAPTURE_TRACKS: usize = 16;
/// Both GPU transfer storage and producer admission share these finite live readback budgets.
pub const MAX_READBACKS: usize = 3;
pub const MAX_READBACK_BYTES: usize = 96 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CapturePhase {
    #[default]
    Idle,
    Armed,
    Starting,
    Recording,
    Finishing,
    Complete,
    Failed,
}

impl CapturePhase {
    /// Finishing retains ownership until readbacks and file workers have completed.
    pub const fn busy(self) -> bool {
        matches!(
            self,
            Self::Armed | Self::Starting | Self::Recording | Self::Finishing
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CaptureStatus {
    pub phase: CapturePhase,
    pub elapsed: MediaTime,
    pub captured_frames: u64,
    pub missed_frames: u64,
    /// The requested slot cadence; missed slots repeat the preceding encoded frame.
    pub fps: FrameRate,
    pub audio: bool,
    pub path: Option<PathBuf>,
    pub message: Option<String>,
    pub health: CaptureHealth,
}

/// Live pipeline observations; latency is elapsed request time, separate from producer CPU work.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CaptureHealth {
    /// Last admitted received-actor preparation, before the capture admission update.
    pub actor_prepare_micros: u64,
    /// Last admitted main-thread sample and GPU request preparation; excludes actor preparation above.
    pub sample_micros: u64,
    /// Last successful request through CPU pixel delivery, including GPU and task scheduling latency.
    pub readback_micros: u64,
    pub pending_readbacks: u32,
    /// Reserved tightly packed pixel bytes, including completed frames waiting for encoder admission.
    pub readback_bytes: u64,
    pub encoder_frames: u32,
    pub encoder_bytes: u64,
    /// Failed nonblocking admission and retry attempts; missed_frames counts actual missing slots.
    pub encoder_stalls: u64,
    /// Capture slots blocked by full readbacks or skipped before a rendered image was available.
    pub readback_stalls: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureRequest {
    Start(CaptureSettings),
    Stop,
    Bookmark,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureSettings {
    #[serde(default)]
    pub mode: CaptureMode,
    pub fps: FrameRate,
    /// Missing fields retain native-sized legacy captures; new preferences use 1080p.
    #[serde(default)]
    pub size: CaptureSize,
    pub codec: ExportCodec,
    pub encoder: Encoder,
    pub quality: u8,
    pub game_audio: bool,
    /// Prepared first-person hand geometry is optional; older take settings leave it disabled.
    #[serde(default)]
    pub hand: bool,
    /// Includes screen HUD/chat in video pixels and the native replay layer only when opted in.
    #[serde(default)]
    pub hud: bool,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            mode: CaptureMode::ReplayOnly,
            fps: FrameRate::default(),
            size: CaptureSize::P1080,
            codec: ExportCodec::H264,
            encoder: Encoder::Auto,
            quality: 20,
            game_audio: true,
            hand: false,
            hud: false,
        }
    }
}

/// Native scene capture can omit the live video encoder and its GPU pixel transfers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    ReplayOnly,
    /// Missing metadata fields describe legacy takes that always encoded a POV video.
    #[default]
    ReplayAndVideo,
}

impl CaptureMode {
    /// User-facing names distinguish editable 3D data from the optional recorded camera video.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ReplayOnly => "3D replay",
            Self::ReplayAndVideo => "3D replay + video",
        }
    }

    /// Both choices retain the same scene and audio data.
    pub const fn next(self) -> Self {
        match self {
            Self::ReplayOnly => Self::ReplayAndVideo,
            Self::ReplayAndVideo => Self::ReplayOnly,
        }
    }

    /// Whether this take needs FFmpeg and GPU readback during gameplay.
    pub const fn records_video(self) -> bool {
        matches!(self, Self::ReplayAndVideo)
    }
}

impl CaptureSettings {
    /// Live recording shares the same rational rate and quality admission as offline export.
    pub fn validate(self) -> Result<(), String> {
        self.fps.validate().map_err(|error| error.to_string())?;
        if self.quality > 51 {
            return Err("Recording quality must be between zero and 51.".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
