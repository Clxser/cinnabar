//! Rational device-to-project clocks and bounded pitch-preserving monitoring of the complete saved mix.

use super::*;
use crate::{
    PlaybackRate, Project,
    export::Cancellation,
    retime::pcm::{FRAME_BYTES, PitchWarp},
};
use std::{
    io::{self, Read},
    path::Path,
    sync::Arc,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MonitorClock {
    origin: MediaTime,
    end: MediaTime,
    speed: PlaybackRate,
    rate: u32,
}

impl MonitorClock {
    /// Device sample indexes start at zero for each seek/rate epoch; all exposed positions are project time.
    pub fn new(
        origin: MediaTime,
        end: MediaTime,
        speed: PlaybackRate,
        rate: u32,
    ) -> Result<Self, String> {
        speed.validate().map_err(|error| error.to_string())?;
        sample_frame(end, rate)?;
        if origin > end {
            return Err("Monitoring starts beyond the project.".into());
        }
        Ok(Self {
            origin,
            end,
            speed,
            rate,
        })
    }

    /// A ceiling is rounded down so the device cannot play beyond an admitted visual timestamp.
    pub fn frame(self, at: MediaTime) -> u64 {
        if at >= self.end {
            return self.end_frame();
        }
        (u128::from(at.saturating_sub(self.origin).0)
            * u128::from(self.rate)
            * u128::from(self.speed.denominator)
            / (u128::from(MICROS_PER_SECOND) * u128::from(self.speed.numerator))) as u64
    }

    /// The final partial sample is admitted once and reports the exact project endpoint.
    pub fn end_frame(self) -> u64 {
        (u128::from(self.end.saturating_sub(self.origin).0)
            * u128::from(self.rate)
            * u128::from(self.speed.denominator))
        .div_ceil(u128::from(MICROS_PER_SECOND) * u128::from(self.speed.numerator)) as u64
    }

    /// Sample boundaries round up so played audio is never reported behind its exact rational clock.
    pub fn time(self, frame: u64) -> MediaTime {
        let delta =
            (u128::from(frame) * u128::from(MICROS_PER_SECOND) * u128::from(self.speed.numerator))
                .div_ceil(u128::from(self.rate) * u128::from(self.speed.denominator));
        MediaTime((u128::from(self.origin.0) + delta).min(u128::from(self.end.0)) as u64)
    }
}

enum Input {
    Direct(Mixer),
    Stretched(PitchWarp<MixedInput>),
}

pub struct MonitorMixer {
    input: Input,
    source_start: u64,
    next: u64,
    end: u64,
}

impl MonitorMixer {
    /// Saved clip rate, pitch policy, gain and fades are mixed first; monitoring stretches that final mix.
    pub fn new(
        project: Arc<Project>,
        base: &Path,
        clock: MonitorClock,
        cancellation: Cancellation,
    ) -> Result<Self, String> {
        let source_start = sample_frame(clock.origin, clock.rate)?;
        let mixer = Mixer::new(project, base, clock.rate, source_start)?;
        let input = if clock.speed == PlaybackRate::default() {
            Input::Direct(mixer)
        } else {
            Input::Stretched(PitchWarp::monitoring(
                MixedInput {
                    mixer,
                    cursor: source_start,
                    cancellation,
                    frames: vec![[0.0; CHANNELS]; 1024],
                    bytes: Vec::new(),
                    offset: 0,
                    padding: PitchWarp::<io::Empty>::padding_frames(clock.rate)? as u64,
                },
                clock.speed,
                clock.rate,
            )?)
        };
        Ok(Self {
            input,
            source_start,
            next: 0,
            end: clock.end_frame(),
        })
    }

    pub const fn end_frame(&self) -> u64 {
        self.end
    }

    /// Every block is contiguous in the device clock, including slow monitoring of long projects.
    pub fn read(
        &mut self,
        output: &mut [[f32; CHANNELS]],
        start: u64,
        cancellation: &Cancellation,
    ) -> Result<(), String> {
        if start != self.next
            || output.len() > MAX_BLOCK_FRAMES
            || start
                .checked_add(output.len() as u64)
                .is_none_or(|end| end > self.end)
        {
            return Err("Monitoring requires bounded contiguous device blocks.".into());
        }
        cancellation.check()?;
        match &mut self.input {
            Input::Direct(mixer) => {
                let begin = self.source_start + start;
                let len = mixer
                    .end_frame()
                    .saturating_sub(begin)
                    .min(output.len() as u64) as usize;
                if len > 0 {
                    mixer.read(&mut output[..len], begin, cancellation)?;
                }
                output[len..].fill([0.0; CHANNELS]);
            }
            Input::Stretched(warp) => {
                for (index, frame) in output.iter_mut().enumerate() {
                    if index % 256 == 0 {
                        cancellation.check()?;
                    }
                    *frame = warp.sample(start + index as u64)?;
                }
            }
        }
        self.next += output.len() as u64;
        Ok(())
    }

    /// Decoder retirement never discards already queued device PCM.
    pub fn retire_decoders(&mut self) {
        match &mut self.input {
            Input::Direct(mixer) => mixer.retire_decoders(),
            Input::Stretched(warp) => warp.input_mut().mixer.retire_decoders(),
        }
    }
}

struct MixedInput {
    mixer: Mixer,
    cursor: u64,
    cancellation: Cancellation,
    frames: Vec<[f32; CHANNELS]>,
    bytes: Vec<u8>,
    offset: usize,
    padding: u64,
}

impl Read for MixedInput {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.cancellation.check().map_err(io::Error::other)?;
        if output.is_empty() {
            return Ok(0);
        }
        if self.offset == self.bytes.len() {
            let remaining = self.mixer.end_frame().saturating_sub(self.cursor);
            let len = remaining.min(self.frames.len() as u64) as usize;
            let len = if len > 0 {
                self.mixer
                    .read(&mut self.frames[..len], self.cursor, &self.cancellation)
                    .map_err(io::Error::other)?;
                self.cursor += len as u64;
                len
            } else {
                let len = self.padding.min(self.frames.len() as u64) as usize;
                self.frames[..len].fill([0.0; CHANNELS]);
                self.padding -= len as u64;
                len
            };
            self.bytes.clear();
            self.bytes.reserve(len * FRAME_BYTES);
            for frame in &self.frames[..len] {
                for channel in frame {
                    self.bytes.extend_from_slice(&channel.to_le_bytes());
                }
            }
            self.offset = 0;
        }
        let count = output.len().min(self.bytes.len() - self.offset);
        output[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn monitor_clock_is_rational_bounded_and_never_plays_ahead_of_visual_credit() {
        for &speed in crate::playback::monitoring::MonitoringSpeed::ALL {
            let clock = MonitorClock::new(
                MediaTime(123_456),
                MediaTime(crate::time::MAX_MEDIA_MICROS),
                speed.rate(),
                48_000,
            )
            .unwrap();
            for at in [
                clock.origin,
                MediaTime(1_000_001),
                MediaTime(86_400_000_000),
            ] {
                let frame = clock.frame(at);
                assert!(clock.time(frame) <= at);
                assert!(clock.time(frame + 1) > at);
            }
            assert_eq!(clock.time(clock.end_frame()), clock.end);
        }
    }

    #[test]
    fn silent_monitoring_stretches_bounded_blocks_without_modifying_the_project() {
        let mut project = Project::new("monitor");
        project.sources.push(crate::model::MediaSource {
            id: crate::Id(1),
            name: "missing".into(),
            path: "not-opened.wav".into(),
            kind: crate::model::MediaKind::Audio,
            duration: MediaTime(50_000),
            size: [0, 0],
            frame_rate: None,
            sha256: None,
        });
        project.audio.push(crate::model::AudioClip {
            id: crate::Id(2),
            source: crate::Id(1),
            linked_clip: None,
            source_range: crate::Span::new(MediaTime::ZERO, MediaTime(50_000)).unwrap(),
            at: MediaTime::ZERO,
            rate: PlaybackRate::default(),
            speed_ramp: None,
            role: crate::model::AudioRole::Game,
            gain: 1.0,
            fade_in: MediaTime::ZERO,
            fade_out: MediaTime::ZERO,
            muted: true,
        });
        let before = project.to_json().unwrap();
        let project = Arc::new(project);
        for &speed in crate::playback::monitoring::MonitoringSpeed::ALL {
            let clock = MonitorClock::new(MediaTime(123), project.duration(), speed.rate(), 48_000)
                .unwrap();
            let cancellation = Cancellation::default();
            let mut mixer =
                MonitorMixer::new(project.clone(), Path::new("."), clock, cancellation.clone())
                    .unwrap();
            let mut cursor = 0;
            while cursor < mixer.end_frame() {
                let len = (mixer.end_frame() - cursor).min(257) as usize;
                let mut output = vec![[1.0; CHANNELS]; len];
                mixer.read(&mut output, cursor, &cancellation).unwrap();
                assert!(output.iter().all(|sample| *sample == [0.0; CHANNELS]));
                cursor += len as u64;
            }
            cancellation.cancel();
            assert!(mixer.read(&mut [], cursor, &cancellation).is_err());
            mixer.retire_decoders();
        }
        assert_eq!(project.to_json().unwrap(), before);
    }
}
