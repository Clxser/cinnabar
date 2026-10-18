//! Original bounded waveform overlap: absolute TimeMap anchors choose grains; correlation preserves phase.

use super::*;
use crate::export::Cancellation;

pub struct PitchWarp<R> {
    input: BufReader<R>,
    map: PitchClock,
    origin: MediaTime,
    rate: u32,
    next: u64,
    grain: usize,
    hop: usize,
    search: usize,
    source: VecDeque<[f32; CHANNELS]>,
    source_next: u64,
    tail: Vec<[f32; CHANNELS]>,
    output: Vec<[f32; CHANNELS]>,
    offset: usize,
    first: bool,
}

enum PitchClock {
    Clip(TimeMap),
    Monitoring(PlaybackRate),
}

impl<R: Read> PitchWarp<R> {
    /// Roughly twenty-millisecond grains bound pitch processing and its nonaccumulating alignment window.
    fn grain_frames(rate: u32) -> Result<usize, String> {
        if rate == 0 || rate > crate::audio::MAX_SAMPLE_RATE {
            return Err("The pitch-preserving PCM rate is unsupported.".into());
        }
        Ok((rate.div_ceil(50) as usize).next_power_of_two().max(32))
    }

    /// Decoder padding covers a complete grain and its phase search after the retained source boundary.
    pub fn padding_frames(rate: u32) -> Result<usize, String> {
        let grain = Self::grain_frames(rate)?;
        Ok(grain + grain / 8 + 4)
    }

    /// The first sample is the requested absolute output index, including a seek into an existing ramp.
    pub fn new(
        input: R,
        map: TimeMap,
        origin: MediaTime,
        rate: u32,
        start: u64,
    ) -> Result<Self, String> {
        Self::from_clock(input, PitchClock::Clip(map), origin, rate, start)
    }

    /// Both clock owners use the same bounded overlap buffers and waveform phase search.
    fn from_clock(
        input: R,
        map: PitchClock,
        origin: MediaTime,
        rate: u32,
        start: u64,
    ) -> Result<Self, String> {
        let grain = Self::grain_frames(rate)?;
        let hop = grain / 2;
        Ok(Self {
            input: BufReader::new(input),
            map,
            origin,
            rate,
            next: start,
            grain,
            hop,
            search: hop / 4,
            source: VecDeque::with_capacity(grain * 3),
            source_next: 0,
            tail: vec![[0.0; CHANNELS]; hop],
            output: vec![[0.0; CHANNELS]; hop],
            offset: hop,
            first: true,
        })
    }

    /// Stretches an already mixed forward PCM stream at unity pitch, independent of saved clip maps.
    pub fn monitoring(input: R, speed: PlaybackRate, rate: u32) -> Result<Self, String> {
        speed.validate().map_err(|error| error.to_string())?;
        Self::from_clock(
            input,
            PitchClock::Monitoring(speed),
            MediaTime::ZERO,
            rate,
            0,
        )
    }

    /// EOF retirement can release source decoders while already produced output drains on the device.
    pub fn input_mut(&mut self) -> &mut R {
        self.input.get_mut()
    }

    /// Anchors are recomputed from the shared map every grain; no prior grain's phase offset changes time.
    pub fn sample(&mut self, at: u64) -> Result<[f32; CHANNELS], String> {
        if at != self.next {
            return Err("Pitch-preserving audio requires contiguous output samples.".into());
        }
        if self.offset == self.hop {
            self.prepare()?;
            self.offset = 0;
        }
        let value = self.output[self.offset];
        self.offset += 1;
        self.next += 1;
        Ok(value)
    }

    /// A seek or clip boundary owns the decoder; draining never retains or allocates the remaining PCM.
    pub fn drain(&mut self, cancellation: &Cancellation) -> Result<(), String> {
        let mut bytes = [0; 8192];
        let mut retained = 0;
        loop {
            cancellation.check()?;
            let count = self
                .input
                .read(&mut bytes)
                .map_err(|error| error.to_string())?;
            if count == 0 {
                return Ok(());
            }
            retained += count;
            if retained > self.grain * (crate::time::MAX_PLAYBACK_RATE as usize + 2) * FRAME_BYTES {
                return Err("Pitch-preserving audio exceeded its bounded source tail.".into());
            }
        }
    }

    /// Phase alignment searches only a quarter hop around the exact source anchor, then overlaps at unity pitch.
    fn prepare(&mut self) -> Result<(), String> {
        let source = match &self.map {
            PitchClock::Clip(map) => {
                map.sample_clock(self.next, self.rate)
                    .map_err(|error| error.to_string())?
                    .0
            }
            PitchClock::Monitoring(speed) => {
                self.next as f64
                    * crate::time::MICROS_PER_SECOND as f64
                    * f64::from(speed.numerator)
                    / (f64::from(self.rate) * f64::from(speed.denominator))
            }
        };
        let position = (source - self.origin.0 as f64).max(0.0) * f64::from(self.rate)
            / crate::time::MICROS_PER_SECOND as f64;
        let nominal = position.floor() as i64;
        let fraction = position.fract();
        let required = nominal
            .saturating_add((self.search + self.grain + 2) as i64)
            .max(0) as u64;
        let maximum = self.grain * 3;
        while self.source_next <= required {
            let frame = read_frame(&mut self.input)?;
            if self.source.len() == maximum {
                self.source.pop_front();
            }
            self.source.push_back(frame);
            self.source_next += 1;
        }
        let mut chosen = nominal;
        if !self.first {
            let low = nominal.saturating_sub(self.search as i64).max(0);
            let high = nominal.saturating_add(self.search as i64);
            let step = (self.search / 32).max(1);
            let mut best = self.score(nominal)?;
            for candidate in (low..=high).step_by(step) {
                let score = self.score(candidate)?;
                if score > best + 1e-9
                    || (score - best).abs() <= 1e-9
                        && (candidate - nominal).abs() < (chosen - nominal).abs()
                {
                    best = score;
                    chosen = candidate;
                }
            }
            let centre = chosen;
            let refinement = step.saturating_sub(1) as i64;
            for candidate in centre.saturating_sub(refinement).max(low)
                ..=centre.saturating_add(refinement).min(high)
            {
                let score = self.score(candidate)?;
                if score > best + 1e-9
                    || (score - best).abs() <= 1e-9
                        && (candidate - nominal).abs() < (chosen - nominal).abs()
                {
                    best = score;
                    chosen = candidate;
                }
            }
        }
        for index in 0..self.hop {
            let current = self.interpolated(chosen + index as i64, fraction)?;
            let next = self.interpolated(chosen + self.hop as i64 + index as i64, fraction)?;
            let blend = if self.first {
                1.0
            } else {
                0.5 - 0.5 * (std::f64::consts::PI * index as f64 / self.hop as f64).cos()
            } as f32;
            self.output[index] = std::array::from_fn(|channel| {
                self.tail[index][channel] * (1.0 - blend) + current[channel] * blend
            });
            self.tail[index] = next;
        }
        self.first = false;
        Ok(())
    }

    /// Stereo correlation avoids cancelling antiphase content into an unusable mono reference.
    fn score(&self, start: i64) -> Result<f64, String> {
        let (mut dot, mut left, mut right) = (0.0, 0.0, 0.0);
        for index in (0..self.hop).step_by((self.hop / 64).max(1)) {
            let candidate = self.frame(start + index as i64)?;
            for channel in 0..CHANNELS {
                let a = f64::from(self.tail[index][channel]);
                let b = f64::from(candidate[channel]);
                dot += a * b;
                left += a * a;
                right += b * b;
            }
        }
        let energy = (left * right).sqrt();
        Ok(if energy > 1e-20 { dot / energy } else { -1.0 })
    }

    /// Fractional input seeks retain their sub-sample phase without changing each grain's pitch.
    fn interpolated(&self, index: i64, fraction: f64) -> Result<[f32; CHANNELS], String> {
        let a = self.frame(index)?;
        let b = self.frame(index + 1)?;
        Ok(std::array::from_fn(|channel| {
            (f64::from(a[channel]) * (1.0 - fraction) + f64::from(b[channel]) * fraction) as f32
        }))
    }

    /// The ring retains at most three grains even while fast playback skips large portions of source PCM.
    fn frame(&self, at: i64) -> Result<[f32; CHANNELS], String> {
        if at < 0 {
            return Ok([0.0; CHANNELS]);
        }
        let first = self.source_next.saturating_sub(self.source.len() as u64);
        let index = (at as u64)
            .checked_sub(first)
            .ok_or("The pitch-preserving source window expired.")?;
        self.source
            .get(index as usize)
            .copied()
            .ok_or_else(|| "The pitch-preserving source window is unavailable.".into())
    }
}

#[cfg(test)]
mod tests;
