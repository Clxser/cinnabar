//! Monitoring policy is runtime-only: it never changes saved clip rates or export timing.

use crate::{
    MediaTime, PlaybackRate, StudioError,
    time::{MAX_MEDIA_MICROS, MICROS_PER_SECOND},
};

pub const MAX_SEEK_BYTES: usize = 32;
pub const JUMP_SECONDS: i64 = 5;

macro_rules! speeds {
    ($($name:ident => ($label:literal, $n:literal, $d:literal)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum MonitoringSpeed { $($name),+ }
        impl MonitoringSpeed {
            pub const ALL: &[Self] = &[$(Self::$name),+];
            /// UI labels and exact clock multipliers share one definition.
            pub const fn label(self) -> &'static str { match self { $(Self::$name => $label),+ } }
            pub const fn rate(self) -> PlaybackRate { match self { $(Self::$name => PlaybackRate { numerator:$n, denominator:$d }),+ } }
        }
    };
}
speeds! {
    Tenth => ("0.1x", 1, 10), Quarter => ("0.25x", 1, 4), Half => ("0.5x", 1, 2),
    ThreeQuarters => ("0.75x", 3, 4), Normal => ("1x", 1, 1),
    OneAndHalf => ("1.5x", 3, 2), Double => ("2x", 2, 1), Quadruple => ("4x", 4, 1), Eight => ("8x", 8, 1),
}

impl Default for MonitoringSpeed {
    fn default() -> Self {
        Self::Normal
    }
}

impl MonitoringSpeed {
    /// Increment buttons stop at the supported endpoints instead of wrapping unexpectedly.
    pub fn stepped(self, direction: i8) -> Self {
        let index = Self::ALL
            .iter()
            .position(|speed| *speed == self)
            .expect("registered monitoring speed");
        Self::ALL[(index as isize + direction.signum() as isize)
            .clamp(0, Self::ALL.len() as isize - 1) as usize]
    }

    /// Fast monitoring samples fewer project-time pictures to retain the same wall-time display budget.
    pub fn cadence(self, original: crate::FrameRate) -> Result<crate::FrameRate, StudioError> {
        original.validate()?;
        let speed = self.rate();
        if speed.numerator <= speed.denominator {
            return Ok(original);
        }
        let mut numerator = u64::from(original.numerator) * u64::from(speed.denominator);
        let mut denominator = u64::from(original.denominator) * u64::from(speed.numerator);
        if numerator < denominator {
            return Ok(crate::FrameRate {
                numerator: 1,
                denominator: 1,
            });
        }
        let (mut a, mut b) = (numerator, denominator);
        while b != 0 {
            let remainder = a % b;
            a = b;
            b = remainder;
        }
        numerator /= a;
        denominator /= a;
        let result = crate::FrameRate {
            numerator: u32::try_from(numerator).map_err(|_| StudioError::TimeOverflow)?,
            denominator: u32::try_from(denominator).map_err(|_| StudioError::TimeOverflow)?,
        };
        result.validate()?;
        Ok(result)
    }
}

/// Whole-second jumps clamp before conversion, including reverse jumps near the beginning.
pub fn jumped(at: MediaTime, seconds: i64, duration: MediaTime) -> MediaTime {
    MediaTime(
        (i128::from(at.0) + i128::from(seconds) * i128::from(MICROS_PER_SECOND))
            .clamp(0, i128::from(duration.0)) as u64,
    )
}

/// Admits decimal seconds, mm:ss.ffffff and hh:mm:ss.ffffff without floating-point rounding.
pub fn parse_seek(value: &str) -> Result<MediaTime, StudioError> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_SEEK_BYTES {
        return Err(StudioError::Invalid("Enter seconds, mm:ss or hh:mm:ss."));
    }
    let fields: Vec<_> = value.split(':').collect();
    if fields.len() > 3 {
        return Err(StudioError::Invalid("Use seconds, mm:ss or hh:mm:ss."));
    }
    let mut seconds = 0_u64;
    let mut micros = 0;
    for (index, field) in fields.iter().enumerate() {
        let (whole, fraction) = field
            .split_once('.')
            .map_or((*field, None), |(a, b)| (a, Some(b)));
        if whole.is_empty()
            || !whole.bytes().all(|b| b.is_ascii_digit())
            || fraction.is_some() && index + 1 != fields.len()
        {
            return Err(StudioError::Invalid(
                "Timecode must contain nonnegative decimal numbers.",
            ));
        }
        let part: u64 = whole.parse().map_err(|_| StudioError::TimeOverflow)?;
        if index > 0 && part >= 60 {
            return Err(StudioError::Invalid(
                "Timecode minutes and seconds must be below 60.",
            ));
        }
        seconds = seconds
            .checked_mul(60)
            .and_then(|n| n.checked_add(part))
            .ok_or(StudioError::TimeOverflow)?;
        if let Some(fraction) = fraction {
            if fraction.is_empty()
                || fraction.len() > 6
                || !fraction.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(StudioError::Invalid(
                    "Use at most six fractional second digits.",
                ));
            }
            micros = fraction
                .parse::<u64>()
                .map_err(|_| StudioError::TimeOverflow)?
                * 10_u64.pow(6 - fraction.len() as u32);
        }
    }
    seconds
        .checked_mul(MICROS_PER_SECOND)
        .and_then(|n| n.checked_add(micros))
        .filter(|n| *n <= MAX_MEDIA_MICROS)
        .map(MediaTime)
        .ok_or(StudioError::TimeOverflow)
}

/// Editing shows every retained microsecond; presentation labels may use fewer decimal places.
pub fn seek_text(at: MediaTime) -> String {
    format!(
        "{:02}:{:02}:{:02}.{:06}",
        at.0 / MICROS_PER_SECOND / 3600,
        at.0 / MICROS_PER_SECOND / 60 % 60,
        at.0 / MICROS_PER_SECOND % 60,
        at.0 % MICROS_PER_SECOND
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_timecodes_reject_ambiguous_or_unbounded_input() {
        for at in [
            MediaTime::ZERO,
            MediaTime(123_456_789),
            MediaTime(MAX_MEDIA_MICROS),
        ] {
            assert_eq!(parse_seek(&seek_text(at)).unwrap(), at);
        }
        assert_eq!(parse_seek("1:02.000001").unwrap(), MediaTime(62_000_001));
        assert_eq!(parse_seek("3600.123456").unwrap(), MediaTime(3_600_123_456));
        for value in [
            "",
            "-1",
            "NaN",
            "1e3",
            "1:60",
            "1.1:02",
            "1:2:3:4",
            "0.1234567",
            "99999999999999999999999999999999",
        ] {
            assert!(parse_seek(value).is_err(), "{value}");
        }
        assert_eq!(
            jumped(MediaTime(1), -JUMP_SECONDS, MediaTime(9)),
            MediaTime::ZERO
        );
        assert_eq!(
            jumped(MediaTime(1), JUMP_SECONDS, MediaTime(9)),
            MediaTime(9)
        );
    }
    #[test]
    fn monitoring_changes_invalidate_audio_and_loop_on_both_clocks() {
        use super::super::Transport;
        let duration = MediaTime(1_000_000);
        for audible in [false, true] {
            let mut transport = Transport::default();
            transport
                .set_loop(
                    Some(crate::Span::new(MediaTime::ZERO, duration).unwrap()),
                    duration,
                )
                .unwrap();
            transport.play(duration);
            let epoch = transport.epoch();
            transport.set_rate(MonitoringSpeed::Eight.rate()).unwrap();
            assert_ne!(epoch, transport.epoch());
            let epoch = transport.epoch();
            if audible {
                transport.synchronize_audio(duration, duration, duration);
            } else {
                transport.advance(MediaTime(125_000), duration);
            }
            assert_eq!(transport.position(), MediaTime::ZERO);
            assert!(transport.playing());
            assert_ne!(epoch, transport.epoch());
        }
        for &speed in MonitoringSpeed::ALL {
            speed.rate().validate().unwrap();
        }
    }

    #[test]
    fn fast_monitoring_retains_the_display_budget_with_an_exact_rational_cadence() {
        let ntsc = crate::FrameRate {
            numerator: 30_000,
            denominator: 1001,
        };
        for &speed in MonitoringSpeed::ALL {
            let cadence = speed.cadence(ntsc).unwrap();
            let multiplier = speed.rate();
            if multiplier.numerator > multiplier.denominator {
                assert_eq!(
                    u128::from(cadence.numerator)
                        * u128::from(multiplier.numerator)
                        * u128::from(ntsc.denominator),
                    u128::from(ntsc.numerator)
                        * u128::from(cadence.denominator)
                        * u128::from(multiplier.denominator)
                );
            } else {
                assert_eq!(cadence, ntsc);
            }
        }
    }
}
