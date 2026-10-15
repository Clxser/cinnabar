//! Integer media time and rational frame/rate conversion, independent of wall clocks.

use serde::{Deserialize, Serialize};

use crate::StudioError;

pub const MICROS_PER_SECOND: u64 = 1_000_000;
pub const MAX_MEDIA_MICROS: u64 = 7 * 24 * 60 * 60 * MICROS_PER_SECOND;

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MediaTime(pub u64);

impl MediaTime {
    pub const ZERO: Self = Self(0);

    /// Converts seconds at an input edge, rejecting non-finite or out-of-range values.
    pub fn from_seconds(seconds: f64) -> Result<Self, StudioError> {
        let micros = seconds * MICROS_PER_SECOND as f64;
        if !micros.is_finite() || !(0.0..=MAX_MEDIA_MICROS as f64).contains(&micros) {
            return Err(StudioError::TimeOverflow);
        }
        Ok(Self(micros.round() as u64))
    }

    /// Seconds are a sampling value; the saved timeline retains integer microseconds.
    pub fn seconds(self) -> f64 {
        self.0 as f64 / MICROS_PER_SECOND as f64
    }

    /// Adds timeline lengths without wrapping into an unrelated point in the project.
    pub fn checked_add(self, duration: Self) -> Result<Self, StudioError> {
        self.0
            .checked_add(duration.0)
            .filter(|&time| time <= MAX_MEDIA_MICROS)
            .map(Self)
            .ok_or(StudioError::TimeOverflow)
    }

    /// A bounded difference for clamped UI and transport sampling.
    pub const fn saturating_sub(self, earlier: Self) -> Self {
        Self(self.0.saturating_sub(earlier.0))
    }
}

/// Half-open interval; an exact end belongs to the following clip.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Span {
    pub start: MediaTime,
    pub end: MediaTime,
}

impl Span {
    /// Constructs a nonempty, bounded half-open interval.
    pub fn new(start: MediaTime, end: MediaTime) -> Result<Self, StudioError> {
        let result = Self { start, end };
        result.validate()?;
        Ok(result)
    }

    /// Applies the same admission to decoded projects as to newly edited spans.
    pub fn validate(self) -> Result<(), StudioError> {
        if self.start >= self.end || self.end.0 > MAX_MEDIA_MICROS {
            return Err(StudioError::Invalid(
                "timeline ranges must be nonempty and within seven days",
            ));
        }
        Ok(())
    }

    pub const fn duration(self) -> MediaTime {
        MediaTime(self.end.0.saturating_sub(self.start.0))
    }
    pub const fn contains(self, time: MediaTime) -> bool {
        self.start.0 <= time.0 && time.0 < self.end.0
    }
    pub const fn overlaps(self, other: Self) -> bool {
        self.start.0 < other.end.0 && other.start.0 < self.end.0
    }
}

/// Rational presentation cadence, including rates such as 30000/1001.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameRate {
    pub numerator: u32,
    pub denominator: u32,
}

impl Default for FrameRate {
    fn default() -> Self {
        Self {
            numerator: 60,
            denominator: 1,
        }
    }
}

impl std::fmt::Display for FrameRate {
    /// Keeps integer rates familiar while retaining exact NTSC and other rational cadences.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.denominator == 1 {
            write!(f, "{}", self.numerator)
        } else {
            write!(f, "{}/{}", self.numerator, self.denominator)
        }
    }
}

impl FrameRate {
    /// Limits export cadence to 1..=240 without requiring an integral frame rate.
    pub fn validate(self) -> Result<(), StudioError> {
        if self.denominator == 0
            || self.numerator < self.denominator
            || u64::from(self.numerator) > u64::from(self.denominator) * 240
        {
            return Err(StudioError::Invalid(
                "frame rate must be between 1 and 240 fps",
            ));
        }
        Ok(())
    }

    /// Computes each export timestamp from its index, avoiding accumulated frame-step error.
    pub fn frame_time(self, frame: u64) -> Result<MediaTime, StudioError> {
        self.validate()?;
        let micros =
            u128::from(frame) * u128::from(self.denominator) * u128::from(MICROS_PER_SECOND)
                / u128::from(self.numerator);
        u64::try_from(micros)
            .ok()
            .filter(|&value| value <= MAX_MEDIA_MICROS)
            .map(MediaTime)
            .ok_or(StudioError::TimeOverflow)
    }

    /// Returns the frame at or preceding an integer media timestamp.
    pub fn frame_at(self, time: MediaTime) -> Result<u64, StudioError> {
        self.validate()?;
        let numerator = (u128::from(time.0) + 1) * u128::from(self.numerator);
        let denominator = u128::from(self.denominator) * u128::from(MICROS_PER_SECOND);
        Ok((numerator.div_ceil(denominator) - 1) as u64)
    }

    /// Number of whole frames required to cover a duration, without accumulated floating-point error.
    pub fn frames_covering(self, duration: MediaTime) -> Result<u64, StudioError> {
        self.validate()?;
        if duration.0 > MAX_MEDIA_MICROS {
            return Err(StudioError::TimeOverflow);
        }
        let numerator = u128::from(duration.0) * u128::from(self.numerator);
        let denominator = u128::from(self.denominator) * u128::from(MICROS_PER_SECOND);
        u64::try_from(numerator.div_ceil(denominator)).map_err(|_| StudioError::TimeOverflow)
    }

    /// Snaps editor actions to the nearest actual presentation frame.
    pub fn snap(self, time: MediaTime) -> Result<MediaTime, StudioError> {
        let frame = self.frame_at(time)?;
        let before = self.frame_time(frame)?;
        let Ok(after) = self.frame_time(frame + 1) else {
            return Ok(before);
        };
        Ok(if time.0 - before.0 <= after.0.saturating_sub(time.0) {
            before
        } else {
            after
        })
    }
}

/// Source seconds consumed per timeline second.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaybackRate {
    pub numerator: u32,
    pub denominator: u32,
}

impl Default for PlaybackRate {
    fn default() -> Self {
        Self {
            numerator: 1,
            denominator: 1,
        }
    }
}

impl PlaybackRate {
    /// Bounds supported speed changes to 1/16x..=16x and excludes zero rates.
    pub fn validate(self) -> Result<(), StudioError> {
        let (n, d) = (u64::from(self.numerator), u64::from(self.denominator));
        if n == 0 || d == 0 || n * 16 < d || n > d * 16 {
            return Err(StudioError::Invalid(
                "playback speed must be between 1/16x and 16x",
            ));
        }
        Ok(())
    }

    /// Scales an offset into its source, using bounded integer arithmetic.
    pub fn source_offset(self, timeline: MediaTime) -> Result<MediaTime, StudioError> {
        self.validate()?;
        scaled(timeline, self.numerator, self.denominator, false)
    }

    /// Rounds a source interval up so its final source sample is never silently truncated.
    pub fn timeline_duration(self, source: MediaTime) -> Result<MediaTime, StudioError> {
        self.validate()?;
        scaled(source, self.denominator, self.numerator, true)
    }
}

/// Widens products before division, then rejects results outside the media-time bound.
fn scaled(
    time: MediaTime,
    numerator: u32,
    denominator: u32,
    ceil: bool,
) -> Result<MediaTime, StudioError> {
    let product = u128::from(time.0) * u128::from(numerator);
    let divisor = u128::from(denominator);
    let micros = if ceil {
        product.div_ceil(divisor)
    } else {
        product / divisor
    };
    u64::try_from(micros)
        .ok()
        .filter(|&value| value <= MAX_MEDIA_MICROS)
        .map(MediaTime)
        .ok_or(StudioError::TimeOverflow)
}
