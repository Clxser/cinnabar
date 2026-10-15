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
mod session;
pub use manifest::{CaptureManifest, CaptureTrack};
pub use playback::{CameraRecording, RecordedView};
pub use session::{CaptureSession, CaptureSummary};

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
pub const AUDIO_FILE: &str = "game.wav";
const MANIFEST_FILE: &str = "capture.json";
const MANIFEST_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_CAPTURE_HIGHLIGHTS: usize = 16_384;
pub const MAX_CAPTURE_TRACKS: usize = 16;

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
    pub audio: bool,
    pub path: Option<PathBuf>,
    pub message: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureRequest {
    Start,
    Stop,
    Bookmark,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureSettings {
    pub fps: FrameRate,
    pub codec: ExportCodec,
    pub encoder: Encoder,
    pub quality: u8,
    pub game_audio: bool,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            fps: FrameRate::default(),
            codec: ExportCodec::H264,
            encoder: Encoder::Auto,
            quality: 20,
            game_audio: true,
        }
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
