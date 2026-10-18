//! Clip mix controls preserve source timing and linkage while committing ordinary audio metadata.

use crate::{MediaTime, StudioError, model::AudioClip};

pub const MAX_GAIN: f64 = 8.0;
const GAIN_STEP: f64 = 0.1;
const FADE_STEP: MediaTime = MediaTime(crate::time::MICROS_PER_SECOND / 10);

macro_rules! audio_parameters {
    ($($kind:ident => $label:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum AudioParameter { $($kind),+ }
        impl AudioParameter {
            pub const ALL: &[Self] = &[$(Self::$kind),+];
            /// One exhaustive definition owns control labels and their native ordering.
            pub const fn label(self) -> &'static str { match self { $(Self::$kind => $label),+ } }
        }
    };
}
audio_parameters! { Gain => "Gain", FadeIn => "Fade in", FadeOut => "Fade out" }

impl AudioParameter {
    /// Gain is linear amplitude; fade values are output-timeline seconds.
    pub fn value(self, clip: &AudioClip) -> f64 {
        match self {
            Self::Gain => clip.gain,
            Self::FadeIn => clip.fade_in.seconds(),
            Self::FadeOut => clip.fade_out.seconds(),
        }
    }

    /// Bounded nudges retain the linked visual clock and clamp fades to the mapped output duration.
    pub fn adjusted(self, clip: &AudioClip, direction: i8) -> Result<AudioClip, StudioError> {
        let mut adjusted = clip.clone();
        let duration = clip.span()?.duration();
        let direction = direction.signum();
        match self {
            Self::Gain => {
                adjusted.gain = (clip.gain + f64::from(direction) * GAIN_STEP).clamp(0.0, MAX_GAIN)
            }
            Self::FadeIn | Self::FadeOut => {
                let value = if self == Self::FadeIn {
                    &mut adjusted.fade_in
                } else {
                    &mut adjusted.fade_out
                };
                value.0 = if direction < 0 {
                    value.0.saturating_sub(FADE_STEP.0)
                } else if direction > 0 {
                    value.0.saturating_add(FADE_STEP.0).min(duration.0)
                } else {
                    value.0
                };
            }
        }
        Ok(adjusted)
    }
}
