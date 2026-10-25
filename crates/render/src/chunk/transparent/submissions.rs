//! Counts the transparent terrain each frame submits, so workload changes can be compared.
//!
//! Enable `render::chunk::transparent::submissions=debug` in `CINNABAR_LOG` to log the
//! per-frame averages every few seconds.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use bevy::prelude::*;
use web_time::Instant;

/// How often the per-frame averages are logged.
const LOG_INTERVAL: Duration = Duration::from_secs(5);

/// Transparent terrain draws and faces, by family. Each draw covers one sub-chunk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::chunk) struct TransparentTally {
    pub(in crate::chunk) water_sub_chunks: u64,
    pub(in crate::chunk) water_faces: u64,
    pub(in crate::chunk) model_sub_chunks: u64,
    pub(in crate::chunk) model_faces: u64,
}

impl TransparentTally {
    /// One sub-chunk draw of `water` liquid faces and `model` model faces.
    pub(in crate::chunk) fn draw(water: u32, model: u32) -> Self {
        Self {
            water_sub_chunks: u64::from(water != 0),
            water_faces: u64::from(water),
            model_sub_chunks: u64::from(model != 0),
            model_faces: u64::from(model),
        }
    }

    fn add(&mut self, other: Self) {
        self.water_sub_chunks += other.water_sub_chunks;
        self.water_faces += other.water_faces;
        self.model_sub_chunks += other.model_sub_chunks;
        self.model_faces += other.model_faces;
    }

    /// The average per frame over `frames` frames, rounded down.
    fn per_frame(self, frames: u64) -> Self {
        let frames = frames.max(1);
        Self {
            water_sub_chunks: self.water_sub_chunks / frames,
            water_faces: self.water_faces / frames,
            model_sub_chunks: self.model_sub_chunks / frames,
            model_faces: self.model_faces / frames,
        }
    }
}

/// What the transparent pass submitted this frame, plus the totals since the last log.
#[derive(Resource, Default)]
pub(in crate::chunk) struct TransparentSubmissions {
    water_sub_chunks: AtomicU64,
    water_faces: AtomicU64,
    model_sub_chunks: AtomicU64,
    model_faces: AtomicU64,
    last_frame: TransparentTally,
    window: TransparentTally,
    window_frames: u64,
    window_start: Option<Instant>,
}

impl TransparentSubmissions {
    /// Adds draws the transparent pass submitted this frame; render commands call it.
    pub(in crate::chunk) fn record(&self, tally: TransparentTally) {
        for (counter, value) in [
            (&self.water_sub_chunks, tally.water_sub_chunks),
            (&self.water_faces, tally.water_faces),
            (&self.model_sub_chunks, tally.model_sub_chunks),
            (&self.model_faces, tally.model_faces),
        ] {
            if value != 0 {
                counter.fetch_add(value, Ordering::Relaxed);
            }
        }
    }

    /// The most recently finished frame's submissions.
    #[cfg(test)]
    pub(in crate::chunk) fn last_frame(&self) -> TransparentTally {
        self.last_frame
    }

    /// Closes this frame's tally and returns the per-frame averages once `LOG_INTERVAL` has
    /// passed since the window that ends at `now` began.
    fn finish_frame(&mut self, now: Instant) -> Option<(u64, TransparentTally)> {
        let take = |counter: &mut AtomicU64| std::mem::take(counter.get_mut());
        self.last_frame = TransparentTally {
            water_sub_chunks: take(&mut self.water_sub_chunks),
            water_faces: take(&mut self.water_faces),
            model_sub_chunks: take(&mut self.model_sub_chunks),
            model_faces: take(&mut self.model_faces),
        };
        self.window.add(self.last_frame);
        self.window_frames += 1;
        let start = *self.window_start.get_or_insert(now);
        if now.duration_since(start) < LOG_INTERVAL {
            return None;
        }
        let frames = std::mem::take(&mut self.window_frames);
        let average = std::mem::take(&mut self.window).per_frame(frames);
        self.window_start = Some(now);
        Some((frames, average))
    }
}

/// Ends the frame's tally after the transparent pass has been encoded.
pub(in crate::chunk) fn finish_transparent_submissions(
    mut submissions: ResMut<TransparentSubmissions>,
) {
    if let Some((frames, average)) = submissions.finish_frame(Instant::now()) {
        bevy::log::debug!(
            frames,
            water_sub_chunks = average.water_sub_chunks,
            water_faces = average.water_faces,
            model_sub_chunks = average.model_sub_chunks,
            model_faces = average.model_faces,
            "transparent terrain submitted per frame"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_keep_their_own_tally_and_average_over_the_log_window() {
        let mut submissions = TransparentSubmissions::default();
        let start = Instant::now();
        submissions.record(TransparentTally::draw(6, 0));
        submissions.record(TransparentTally::draw(2, 3));
        submissions.record(TransparentTally::draw(0, 0));
        assert_eq!(submissions.finish_frame(start), None);
        assert_eq!(
            submissions.last_frame(),
            TransparentTally {
                water_sub_chunks: 2,
                water_faces: 8,
                model_sub_chunks: 1,
                model_faces: 3,
            }
        );
        submissions.record(TransparentTally::draw(4, 1));
        assert_eq!(
            submissions.finish_frame(start + LOG_INTERVAL),
            Some((
                2,
                TransparentTally {
                    water_sub_chunks: 1,
                    water_faces: 6,
                    model_sub_chunks: 1,
                    model_faces: 2,
                }
            ))
        );
        assert_eq!(
            submissions.last_frame(),
            TransparentTally::draw(4, 1),
            "each frame starts from zero"
        );
        assert_eq!(submissions.finish_frame(start + LOG_INTERVAL), None);
        assert_eq!(submissions.last_frame(), TransparentTally::default());
    }
}
