//! Ordered export encoding shares the live encoder policy without a second frame queue.

use super::*;
use crate::{export::Cancellation, media_process::MediaChild};

pub struct VideoWriter {
    process: MediaChild,
    size: [u32; 2],
    next: u64,
    cancellation: Cancellation,
    encoder: Encoder,
}

impl VideoWriter {
    /// Probes the requested hardware policy and starts one exact-frame encoder on the export worker.
    pub fn start(
        settings: &VideoSettings,
        size: [u32; 2],
        options: EncodeOptions,
        cancellation: Cancellation,
    ) -> Result<Self, String> {
        settings.fps.validate().map_err(|error| error.to_string())?;
        options.validate()?;
        if options.speed != EncodingSpeed::Export {
            return Err("Exact export requires the export encoding policy.".into());
        }
        frame_byte_len(size)?;
        if size.iter().any(|side| *side < 2 || side % 2 != 0) {
            return Err("Export dimensions must be even and at least two pixels.".into());
        }
        let executable = find_ffmpeg()?;
        let encoder = options.select_encoder(&executable, settings.codec)?;
        cancellation.check()?;
        let mut command = Command::new(executable);
        command
            .args(options::arguments(
                settings,
                size,
                PixelLayout::Rgba,
                &settings.path,
                options,
                encoder,
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let process = MediaChild::start(command, cancellation.clone())?;
        Ok(Self {
            process,
            size,
            next: 0,
            cancellation,
            encoder,
        })
    }

    /// Reports the actual selected encoder, including an Auto fallback to software.
    pub const fn encoder(&self) -> Encoder {
        self.encoder
    }

    /// Export writes each admitted frame once; its producer cannot drop, reorder or resize frames.
    pub fn frame(&mut self, index: u64, size: [u32; 2], rgba8: &[u8]) -> Result<(), String> {
        self.cancellation.check()?;
        if index != self.next || size != self.size || frame_byte_len(size)? != rgba8.len() {
            return Err("An export frame has a wrong index, extent or pixel length.".into());
        }
        self.process
            .stdin
            .as_mut()
            .ok_or("Export encoder input is unavailable.")?
            .write_all(rgba8)
            .map_err(|error| format!("Video encoding stopped: {error}"))?;
        self.cancellation.check()?;
        self.next += 1;
        Ok(())
    }

    /// A completed file requires exactly the planned frame count and a successful encoder flush.
    pub fn finish(self, expected: u64) -> Result<(), String> {
        self.cancellation.check()?;
        if expected == 0 || self.next != expected {
            return Err("Export ended with missing frames.".into());
        }
        self.process.finish()
    }
}
