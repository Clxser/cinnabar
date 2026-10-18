//! Source-anchored speed ramps share one monotone clock across pictures, sound and event evidence.

use serde::{Deserialize, Serialize};

use crate::{MediaTime, PlaybackRate, Span, StudioError, effect::Easing, time::MAX_MEDIA_MICROS};

pub(crate) mod anchors;
mod keys;
pub mod pcm;
mod segment;

use segment::Segment;

pub const MAX_SPEED_KEYS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeedKey {
    /// Absolute source-media time keeps a ramp's phase when its clip is trimmed or split.
    pub at: MediaTime,
    /// A rational multiplier on the clip's overall playback rate.
    pub rate: PlaybackRate,
    /// The incoming transition eases source-time duration per sample, keeping the map positive.
    pub easing: Easing,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeedRamp {
    pub keys: Vec<SpeedKey>,
}

#[derive(Clone, Debug)]
enum Mapping {
    Uniform { numerator: u64, denominator: u64 },
    Ramp { segments: Vec<Segment>, origin: f64 },
}

/// A compiled bounded prefix index makes every frame/sample lookup independent of total project size.
#[derive(Clone, Debug)]
pub struct TimeMap {
    source: Span,
    span: Span,
    mapping: Mapping,
}

impl TimeMap {
    /// Compiles rational rates and analytic eased segment integrals once for an admitted clip revision.
    pub fn new(
        source: Span,
        at: MediaTime,
        rate: PlaybackRate,
        ramp: Option<&SpeedRamp>,
    ) -> Result<Self, StudioError> {
        source.validate()?;
        rate.validate()?;
        let base = (u64::from(rate.numerator), u64::from(rate.denominator));
        let mapping = if let Some(ramp) = ramp {
            ramp.validate(rate)?;
            let first = ramp.keys[0].rate;
            if ramp.keys.iter().all(|key| {
                u64::from(key.rate.numerator) * u64::from(first.denominator)
                    == u64::from(first.numerator) * u64::from(key.rate.denominator)
            }) {
                Mapping::Uniform {
                    numerator: base.0 * u64::from(first.numerator),
                    denominator: base.1 * u64::from(first.denominator),
                }
            } else {
                let segments = segment::compile(rate, &ramp.keys);
                let origin = output_at(&segments, source.start.0 as f64);
                Mapping::Ramp { segments, origin }
            }
        } else {
            Mapping::Uniform {
                numerator: base.0,
                denominator: base.1,
            }
        };
        let duration = match &mapping {
            Mapping::Uniform {
                numerator,
                denominator,
            } => {
                let value = (u128::from(source.duration().0) * u128::from(*denominator))
                    .div_ceil(u128::from(*numerator));
                u64::try_from(value).map_err(|_| StudioError::TimeOverflow)?
            }
            Mapping::Ramp { segments, origin } => {
                micros(output_at(segments, source.end.0 as f64) - origin, true)?
            }
        };
        let span = Span::new(at, at.checked_add(MediaTime(duration.max(1)))?)?;
        Ok(Self {
            source,
            span,
            mapping,
        })
    }

    /// The final source sample is retained by rounding the complete mapped duration upward once.
    pub const fn span(&self) -> Span {
        self.span
    }

    /// Uniform clips retain exact integer-rational sampling, including legacy projects without ramps.
    pub fn source_time(&self, at: MediaTime) -> Result<MediaTime, StudioError> {
        if at.0 > MAX_MEDIA_MICROS {
            return Err(StudioError::TimeOverflow);
        }
        let source = match &self.mapping {
            Mapping::Uniform {
                numerator,
                denominator,
            } => {
                let offset = u128::from(at.saturating_sub(self.span.start).0)
                    * u128::from(*numerator)
                    / u128::from(*denominator);
                u128::from(self.source.start.0)
                    .saturating_add(offset)
                    .min(u128::from(self.source.end.0 - 1)) as u64
            }
            Mapping::Ramp { .. } => self.source_position(at)?.floor() as u64,
        };
        Ok(MediaTime(
            source.clamp(self.source.start.0, self.source.end.0 - 1),
        ))
    }

    /// Fractional source microseconds let the PCM resampler use the same absolute clock as video.
    pub fn source_position(&self, at: MediaTime) -> Result<f64, StudioError> {
        if at.0 > MAX_MEDIA_MICROS {
            return Err(StudioError::TimeOverflow);
        }
        Ok(self
            .position_at(at)
            .clamp(self.source.start.0 as f64, self.source.end.0 as f64))
    }

    /// A rational output sample clock avoids rounding every audio sample down to a whole microsecond.
    pub fn sample_clock(&self, frame: u64, sample_rate: u32) -> Result<(f64, f64), StudioError> {
        if sample_rate == 0 {
            return Err(StudioError::Invalid("PCM sample rate must be positive"));
        }
        let at = frame as f64 * crate::time::MICROS_PER_SECOND as f64 / f64::from(sample_rate);
        if !at.is_finite() || !(0.0..=MAX_MEDIA_MICROS as f64).contains(&at) {
            return Err(StudioError::TimeOverflow);
        }
        let source = self
            .position_at_micros(at)
            .clamp(self.source.start.0 as f64, self.source.end.0 as f64);
        let rate = match &self.mapping {
            Mapping::Uniform {
                numerator,
                denominator,
            } => *numerator as f64 / *denominator as f64,
            Mapping::Ramp { segments, .. } => {
                segment_at(segments, source).inverse_rate(source).recip()
            }
        };
        Ok((source, rate))
    }

    /// Inclusive boundaries support trims and event envelopes without dropping the last retained sample.
    pub fn source_boundary(&self, at: MediaTime) -> Result<MediaTime, StudioError> {
        if at.0 > MAX_MEDIA_MICROS {
            return Err(StudioError::TimeOverflow);
        }
        if at == self.span.start {
            return Ok(self.source.start);
        }
        if at == self.span.end {
            return Ok(self.source.end);
        }
        if let Mapping::Uniform {
            numerator,
            denominator,
        } = &self.mapping
        {
            let delta = i128::from(at.0) - i128::from(self.span.start.0);
            let source = i128::from(self.source.start.0)
                + delta * i128::from(*numerator) / i128::from(*denominator);
            return Ok(MediaTime(
                source.clamp(0, i128::from(MAX_MEDIA_MICROS)) as u64
            ));
        }
        Ok(MediaTime(
            self.position_at(at)
                .floor()
                .clamp(0.0, MAX_MEDIA_MICROS as f64) as u64,
        ))
    }

    /// Transitions continue through real source handles using the ramp's original source-time phase.
    pub fn transition_time(
        &self,
        at: MediaTime,
        duration: MediaTime,
    ) -> Result<MediaTime, StudioError> {
        if at.0 > MAX_MEDIA_MICROS || duration.0 > MAX_MEDIA_MICROS {
            return Err(StudioError::TimeOverflow);
        }
        if duration == MediaTime::ZERO {
            return Err(StudioError::Invalid("transition source has no frames"));
        }
        if let Mapping::Uniform {
            numerator,
            denominator,
        } = &self.mapping
        {
            let delta = i128::from(at.0) - i128::from(self.span.start.0);
            let source = i128::from(self.source.start.0)
                + delta * i128::from(*numerator) / i128::from(*denominator);
            return Ok(MediaTime(source.clamp(0, i128::from(duration.0 - 1)) as u64));
        }
        Ok(MediaTime(
            self.position_at(at)
                .floor()
                .clamp(0.0, duration.0.saturating_sub(1) as f64) as u64,
        ))
    }

    /// A retained source event maps to its current output time after cuts, moves and rate changes.
    pub fn timeline_time(&self, source: MediaTime) -> Result<Option<MediaTime>, StudioError> {
        if !self.source.contains(source) {
            return Ok(None);
        }
        self.timeline_boundary(source)
            .map(|at| Some(MediaTime(at.0.min(self.span.end.0 - 1))))
    }

    /// Boundary mapping is also valid for source handles outside a clip's current trim.
    pub fn timeline_boundary(&self, source: MediaTime) -> Result<MediaTime, StudioError> {
        if source.0 > MAX_MEDIA_MICROS {
            return Err(StudioError::TimeOverflow);
        }
        let offset = match &self.mapping {
            Mapping::Uniform {
                numerator,
                denominator,
            } => {
                let value = (i128::from(source.0) - i128::from(self.source.start.0))
                    * i128::from(*denominator);
                let divisor = i128::from(*numerator);
                value.div_euclid(divisor) + i128::from(value.rem_euclid(divisor) != 0)
            }
            Mapping::Ramp { segments, origin } => {
                (output_at(segments, source.0 as f64) - origin).ceil() as i128
            }
        };
        let value = i128::from(self.span.start.0) + offset;
        if !(0..=i128::from(MAX_MEDIA_MICROS)).contains(&value) {
            return Err(StudioError::TimeOverflow);
        }
        Ok(MediaTime(value as u64))
    }

    /// Instantaneous source speed is positive; varispeed audio uses it to choose its antialiasing cutoff.
    pub fn rate_at(&self, at: MediaTime) -> f64 {
        match &self.mapping {
            Mapping::Uniform {
                numerator,
                denominator,
            } => *numerator as f64 / *denominator as f64,
            Mapping::Ramp { segments, .. } => {
                let source = self.position_at(at);
                let segment = segment_at(segments, source);
                segment.inverse_rate(source).recip()
            }
        }
    }

    /// Constant ramps use the same exact path as ordinary constant-rate clips.
    pub fn is_uniform(&self) -> bool {
        matches!(self.mapping, Mapping::Uniform { .. })
    }
    /// Constant ramps expose their combined rational rate to the shared pitch-preserving decoder.
    pub fn uniform_rate(&self) -> Option<(u64, u64)> {
        match self.mapping {
            Mapping::Uniform {
                numerator,
                denominator,
            } => Some((numerator, denominator)),
            Mapping::Ramp { .. } => None,
        }
    }

    /// This source position is intentionally unclamped to the trim so transitions can use source handles.
    fn position_at(&self, at: MediaTime) -> f64 {
        self.position_at_micros(at.0 as f64)
    }

    /// Inversion takes the requested clock directly and never accumulates elapsed-time error.
    fn position_at_micros(&self, at: f64) -> f64 {
        let delta = at - self.span.start.0 as f64;
        match &self.mapping {
            Mapping::Uniform {
                numerator,
                denominator,
            } => self.source.start.0 as f64 + delta * *numerator as f64 / *denominator as f64,
            Mapping::Ramp { segments, origin } => {
                let output = (origin + delta).clamp(
                    0.0,
                    segments.last().map_or(0.0, |segment| segment.output_end),
                );
                let index = segments
                    .partition_point(|segment| segment.output_end < output)
                    .min(segments.len() - 1);
                segments[index].source_at(output)
            }
        }
    }
}

impl SpeedRamp {
    /// Every saved multiplier and its combined playback rate has one bounded admission contract.
    pub fn validate(&self, base: PlaybackRate) -> Result<(), StudioError> {
        base.validate()?;
        crate::error::limit("speed ramp keys", self.keys.len(), MAX_SPEED_KEYS)?;
        if self.keys.is_empty()
            || self.keys.windows(2).any(|pair| pair[0].at >= pair[1].at)
            || self
                .keys
                .last()
                .is_some_and(|key| key.at.0 > MAX_MEDIA_MICROS)
        {
            return Err(StudioError::Invalid(
                "speed ramp keys must be ordered within the source clock",
            ));
        }
        let maximum = u128::from(crate::time::MAX_PLAYBACK_RATE);
        for key in &self.keys {
            key.rate.validate()?;
            let numerator = u128::from(base.numerator) * u128::from(key.rate.numerator);
            let denominator = u128::from(base.denominator) * u128::from(key.rate.denominator);
            if numerator * maximum < denominator || numerator > denominator * maximum {
                return Err(StudioError::Invalid(
                    "combined clip and ramp speed must remain between 1/16x and 16x",
                ));
            }
        }
        Ok(())
    }
}

/// Prefix indexes choose their segment with binary search, including the source clock's constant tails.
fn segment_at(segments: &[Segment], source: f64) -> &Segment {
    let index = segments
        .partition_point(|segment| segment.end < source)
        .min(segments.len() - 1);
    &segments[index]
}

/// The same integral determines clip duration, event anchors and inverse sample positioning.
fn output_at(segments: &[Segment], source: f64) -> f64 {
    segment_at(segments, source).output_at(source)
}

/// Sampling keeps fractional arithmetic private and rejects non-finite/out-of-range saved media times.
fn micros(value: f64, ceil: bool) -> Result<u64, StudioError> {
    let value = if ceil { value.ceil() } else { value.floor() };
    if !value.is_finite() || !(0.0..=MAX_MEDIA_MICROS as f64).contains(&value) {
        return Err(StudioError::TimeOverflow);
    }
    Ok(value as u64)
}
