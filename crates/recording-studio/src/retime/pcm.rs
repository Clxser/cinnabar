//! Bounded sequential stereo varispeed sampling uses the same absolute time map as rendered pictures.

use std::{
    collections::VecDeque,
    io::{BufReader, Read},
};

use super::*;

pub const CHANNELS: usize = 2;
const SAMPLE_BYTES: usize = std::mem::size_of::<f32>();
pub const FRAME_BYTES: usize = CHANNELS * SAMPLE_BYTES;
const FILTER_RADIUS: usize = 16;
pub const PADDING_FRAMES: usize = FILTER_RADIUS * crate::time::MAX_PLAYBACK_RATE as usize + 2;
const WINDOW_FRAMES: usize = PADDING_FRAMES * 2 + 2;

pub struct StereoWarp<R> {
    input: BufReader<R>,
    map: TimeMap,
    source_origin: MediaTime,
    sample_rate: u32,
    window: VecDeque<[f32; CHANNELS]>,
    next_source: u64,
    last_output: Option<u64>,
}

impl<R: Read> StereoWarp<R> {
    /// Input is little-endian interleaved stereo PCM at `sample_rate`, starting at the retained source trim.
    pub fn new(
        input: R,
        map: TimeMap,
        source_origin: MediaTime,
        sample_rate: u32,
    ) -> Result<Self, String> {
        if sample_rate == 0 {
            return Err("PCM sample rate must be positive.".into());
        }
        Ok(Self {
            input: BufReader::new(input),
            map,
            source_origin,
            sample_rate,
            window: VecDeque::with_capacity(WINDOW_FRAMES),
            next_source: 0,
            last_output: None,
        })
    }

    /// Forward output sample indexes permit dropping old decoder data without changing the absolute source clock.
    pub fn sample(&mut self, frame: u64) -> Result<[f32; CHANNELS], String> {
        if self.last_output.is_some_and(|previous| frame < previous) {
            return Err(
                "PCM playback moved backwards; seek the source decoder before sampling again."
                    .into(),
            );
        }
        let (source, rate) = self
            .map
            .sample_clock(frame, self.sample_rate)
            .map_err(|error| error.to_string())?;
        let position = ((source - self.source_origin.0 as f64).max(0.0)
            * f64::from(self.sample_rate)
            / crate::time::MICROS_PER_SECOND as f64)
            .max(0.0);
        let cutoff = rate.max(1.0).recip();
        let radius = (FILTER_RADIUS as f64 / cutoff).ceil() as i64;
        let centre = position.floor() as i64;
        let required = centre.saturating_add(radius).max(0) as u64;
        self.fill(required)?;
        let first = self.next_source.saturating_sub(self.window.len() as u64);
        let mut sums = [0.0; CHANNELS];
        let mut total = 0.0;
        for index in centre - radius + 1..=centre + radius {
            let distance = index as f64 - position;
            let phase = std::f64::consts::PI * distance * cutoff;
            let sinc = if phase.abs() < 1e-9 {
                1.0
            } else {
                phase.sin() / phase
            };
            let window = 0.5 * (1.0 + (std::f64::consts::PI * distance / radius as f64).cos());
            let weight = cutoff * sinc * window;
            total += weight;
            if index < 0 {
                continue;
            }
            let offset = (index as u64)
                .checked_sub(first)
                .ok_or("PCM source window is no longer retained.")?
                as usize;
            let sample = self
                .window
                .get(offset)
                .ok_or("PCM decoder did not retain the required source sample.")?;
            for channel in 0..CHANNELS {
                sums[channel] += f64::from(sample[channel]) * weight;
            }
        }
        if !total.is_finite() || total.abs() < 1e-12 {
            return Err("PCM speed filter could not form a finite sample.".into());
        }
        let result = sums.map(|value| (value / total) as f32);
        if result.iter().any(|sample| !sample.is_finite()) {
            return Err("Audio contains non-finite sample values.".into());
        }
        self.last_output = Some(frame);
        Ok(result)
    }

    /// The ring retains only the antialiasing window; decoder EOF before its declared padded extent is an error.
    fn fill(&mut self, required: u64) -> Result<(), String> {
        while self.next_source <= required {
            let mut bytes = [0; FRAME_BYTES];
            self.input.read_exact(&mut bytes).map_err(|error| {
                format!("Audio decode ended before its declared source extent: {error}")
            })?;
            let frame = std::array::from_fn(|channel| {
                let offset = channel * SAMPLE_BYTES;
                f32::from_le_bytes(
                    bytes[offset..offset + SAMPLE_BYTES]
                        .try_into()
                        .expect("one PCM sample"),
                )
            });
            if frame.iter().any(|value| !value.is_finite()) {
                return Err("Audio contains non-finite source samples.".into());
            }
            if self.window.len() == WINDOW_FRAMES {
                self.window.pop_front();
            }
            self.window.push_back(frame);
            self.next_source += 1;
        }
        Ok(())
    }
}
