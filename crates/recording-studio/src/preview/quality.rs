//! Interactive render extent is independent of the project's final export settings.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PreviewQuality {
    Fast,
    #[default]
    Balanced,
    Full,
}

impl PreviewQuality {
    /// Interactive playback is paced for display, without shrinking its audio credit at 120/240 FPS.
    pub fn frame_rate(self, output: crate::FrameRate) -> crate::FrameRate {
        let maximum = match self {
            Self::Fast => 30,
            Self::Balanced | Self::Full => 60,
        };
        if u64::from(output.numerator) <= u64::from(output.denominator) * u64::from(maximum) {
            output
        } else {
            crate::FrameRate {
                numerator: maximum,
                denominator: 1,
            }
        }
    }
    /// Short names fit both the viewer selector and the preview settings page.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fast => "Fast",
            Self::Balanced => "Balanced",
            Self::Full => "Full",
        }
    }

    /// Cycles the complete bounded set of interactive rendering choices.
    pub const fn next(self) -> Self {
        match self {
            Self::Fast => Self::Balanced,
            Self::Balanced => Self::Full,
            Self::Full => Self::Fast,
        }
    }

    /// Scales the presentation edge's existing cap; neither media nor export extents are enlarged.
    pub fn maximum_size(self, maximum: [u32; 2]) -> [u32; 2] {
        let numerator = match self {
            Self::Fast => 2,
            Self::Balanced => 3,
            Self::Full => 4,
        };
        maximum.map(|side| ((u64::from(side) * numerator / 4) as u32).max(u32::from(side != 0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_only_reduces_the_callers_preview_cap_and_cycles_all_choices() {
        let mut quality = PreviewQuality::default();
        assert_eq!(quality, PreviewQuality::Balanced);
        assert_eq!(quality.maximum_size([1280, 720]), [960, 540]);
        quality = quality.next();
        assert_eq!(quality.maximum_size([1280, 720]), [1280, 720]);
        quality = quality.next();
        assert_eq!(quality.maximum_size([1280, 720]), [640, 360]);
        assert_eq!(quality.maximum_size([1, 1]), [1, 1]);
        assert_eq!(quality.maximum_size([0, 0]), [0, 0]);
        assert_eq!(quality.next(), PreviewQuality::default());
    }
}
