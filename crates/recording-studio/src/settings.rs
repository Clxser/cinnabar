//! Shared recording and output presets; arbitrary admitted rational rates remain valid in projects.

use crate::{
    FrameRate,
    model::{Encoder, ExportCodec},
};

pub const FPS_PRESETS: &[u32] = &[24, 30, 60, 120, 240];
pub const SIZE_PRESETS: &[[u32; 2]] = &[[1280, 720], [1920, 1080], [2560, 1440]];
pub const MAX_PATH_BYTES: usize = 4096;

/// Advances to the next authored preset, retaining full rational-rate admission elsewhere.
pub fn next_fps(current: FrameRate) -> FrameRate {
    let next = FPS_PRESETS
        .iter()
        .position(|&fps| {
            u64::from(current.numerator) == u64::from(fps) * u64::from(current.denominator)
        })
        .map_or(0, |index| (index + 1) % FPS_PRESETS.len());
    FrameRate {
        numerator: FPS_PRESETS[next],
        denominator: 1,
    }
}

/// UI labels and settings serialization refer to the same admitted codec variants.
pub const fn codec_label(codec: ExportCodec) -> &'static str {
    match codec {
        ExportCodec::H264 => "H.264",
        ExportCodec::Hevc => "HEVC",
        ExportCodec::Av1 => "AV1",
    }
}

/// Automatic encoding probes the driver and can select CPU; explicit NVIDIA requires that device.
pub const fn encoder_label(encoder: Encoder) -> &'static str {
    match encoder {
        Encoder::Auto => "Automatic",
        Encoder::Cpu => "CPU",
        Encoder::Nvidia => "NVIDIA",
    }
}
