//! Worker-local preview diagnostics distinguish source decoding from recorded-video capture misses.

use std::time::{Duration, Instant};

use bevy::log::{info, warn};
use recording_studio::preview::PreviewRequest;

const REPORT_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct PreviewMetrics {
    identity: Option<([u64; 3], bool, usize, usize)>,
    started: Option<Instant>,
    frames: u64,
    skipped: u64,
    previous: Option<u64>,
    total: Duration,
    maximum: Duration,
}

impl PreviewMetrics {
    /// Measures only admitted work on the IO/render worker, without adding synchronization to the UI.
    pub(super) fn frame(
        &mut self,
        request: &PreviewRequest,
        elapsed: Duration,
        native: bool,
        decoders: (usize, usize),
        error: Option<&str>,
    ) {
        let (hardware, software) = decoders;
        let identity = (request.epoch, native, hardware, software);
        let source = if native && hardware + software > 0 {
            "native replay and raster media"
        } else if native {
            "native replay"
        } else {
            "recorded video"
        };
        let now = Instant::now();
        if self.identity != Some(identity) {
            *self = Self {
                identity: Some(identity),
                started: Some(now),
                ..Self::default()
            };
            info!(
                source,
                hardware_decoders = hardware,
                software_decoders = software,
                at_us = request.at.0,
                "Studio preview source initialized"
            );
        }
        if let Some(error) = error {
            warn!(
                source,
                at_us = request.at.0,
                error,
                "Studio preview frame failed"
            );
            return;
        }
        let number = request.frame_rate().frame_at(request.at).unwrap_or(0);
        if let Some(previous) = self.previous {
            self.skipped += number.saturating_sub(previous.saturating_add(1));
        }
        self.previous = Some(number);
        self.frames += 1;
        self.total += elapsed;
        self.maximum = self.maximum.max(elapsed);
        if now.duration_since(self.started.unwrap_or(now)) < REPORT_INTERVAL {
            return;
        }
        info!(
            source,
            frames = self.frames,
            skipped_output_frames = self.skipped,
            mean_prepare_ms = self.total.as_secs_f64() * 1000.0 / self.frames as f64,
            max_prepare_ms = self.maximum.as_secs_f64() * 1000.0,
            at_us = request.at.0,
            "Studio preview throughput"
        );
        self.started = Some(now);
        self.frames = 0;
        self.skipped = 0;
        self.total = Duration::ZERO;
        self.maximum = Duration::ZERO;
    }
}
