//! Shared bounded PCM preparation for real-time preview and offline export.

use crate::{MediaTime, time::MICROS_PER_SECOND};
use std::path::PathBuf;

pub mod beats;
mod decode;
mod mixer;
mod monitoring;
mod parameter;
pub use decode::ClipDecoder;
pub use mixer::{Mixer, add as mix_into};
pub use monitoring::{MonitorClock, MonitorMixer};
pub use parameter::{AudioParameter, MAX_GAIN};

pub const CHANNELS: usize = crate::retime::pcm::CHANNELS;
pub const MAX_BLOCK_FRAMES: usize = 16_384;
pub const MAX_PREVIEW_STREAMS: usize = 32;
pub const MAX_SAMPLE_RATE: u32 = 384_000;

#[derive(Clone)]
pub struct PreviewRequest {
    pub preview: crate::preview::PreviewRequest,
    pub duration: MediaTime,
    pub playing: bool,
    pub ceiling: Option<MediaTime>,
    /// Runtime monitoring never changes the immutable project or its saved clip timing.
    pub speed: crate::playback::monitoring::MonitoringSpeed,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreviewStatus {
    pub epoch: Option<[u64; 3]>,
    /// An engaged hardware sample clock paces the timeline, including buffering stalls.
    pub engaged: bool,
    pub ready: bool,
    pub buffering: bool,
    pub position: MediaTime,
    /// Present only after the device callback confirms the final cursor of a paused output.
    pub paused_position: Option<MediaTime>,
    pub buffered_until: MediaTime,
    pub underruns: u64,
    pub error: Option<String>,
}

/// Ceiling puts half-open clip boundaries on one shared output PCM sample grid.
pub fn sample_frame(at: MediaTime, rate: u32) -> Result<u64, String> {
    if rate == 0 || rate > MAX_SAMPLE_RATE || at.0 > crate::time::MAX_MEDIA_MICROS {
        return Err("The PCM sample clock is outside its supported bounds.".into());
    }
    u64::try_from((u128::from(at.0) * u128::from(rate)).div_ceil(u128::from(MICROS_PER_SECOND)))
        .map_err(|_| "The PCM sample clock overflows.".into())
}

/// The sample cursor reports completed frames without rounding ahead of an audible sample.
pub fn sample_time(frame: u64, rate: u32) -> Result<MediaTime, String> {
    if rate == 0 || rate > MAX_SAMPLE_RATE {
        return Err("The PCM sample rate is unsupported.".into());
    }
    let micros = u128::from(frame) * u128::from(MICROS_PER_SECOND) / u128::from(rate);
    let micros = u64::try_from(micros).map_err(|_| "The PCM sample clock overflows.")?;
    if micros > crate::time::MAX_MEDIA_MICROS + MICROS_PER_SECOND {
        return Err("The PCM sample clock exceeds its media bound.".into());
    }
    Ok(MediaTime(micros.min(crate::time::MAX_MEDIA_MICROS)))
}

/// Media paths stay local; native replay audio is its recorded sidecar rather than the replay directory.
pub fn source_path(source: &crate::model::MediaSource, base: &std::path::Path) -> PathBuf {
    let path = if source.path.is_absolute() {
        source.path.clone()
    } else {
        base.join(&source.path)
    };
    if source.kind == crate::model::MediaKind::Replay {
        path.join(crate::capture::AUDIO_FILE)
    } else {
        path
    }
}

#[cfg(test)]
mod tests;
