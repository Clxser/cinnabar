//! Screen-anchored editorial text is composed after scene effects and uses its own opacity envelope.

use crate::{MediaTime, model::TitleClip};

pub const MAX_TITLE_TEXT_BYTES: usize = 16_384;

macro_rules! parameters {
    ($($name:ident => ($label:literal, $step:literal)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum TitleParameter { $($name),+ }
        impl TitleParameter {
            pub const ALL: &[Self] = &[$(Self::$name),+];
            pub const fn label(self) -> &'static str { match self { $(Self::$name => $label),+ } }
            fn step(self) -> f64 { match self { $(Self::$name => $step),+ } }
        }
    };
}
parameters! {
    X => ("Horizontal position", 0.02), Y => ("Vertical position", 0.02),
    Size => ("Text size", 0.01), Red => ("Red", 0.05), Green => ("Green", 0.05),
    Blue => ("Blue", 0.05), Opacity => ("Opacity", 0.05),
    FadeIn => ("Fade in (seconds)", 0.05), FadeOut => ("Fade out (seconds)", 0.05),
}

impl TitleParameter {
    /// Normalized display values use output coordinates; fade values are seconds.
    pub fn value(self, title: &TitleClip) -> f64 {
        match self {
            Self::X => title.anchor[0],
            Self::Y => title.anchor[1],
            Self::Size => title.size,
            Self::Red => title.color[0],
            Self::Green => title.color[1],
            Self::Blue => title.color[2],
            Self::Opacity => title.color[3],
            Self::FadeIn => title.fade_in.seconds(),
            Self::FadeOut => title.fade_out.seconds(),
        }
    }

    /// Clamps one native adjustment to the same bounds admitted by persisted projects.
    pub fn adjust(self, title: &mut TitleClip, direction: i8, fine: bool) {
        let value = self.value(title)
            + f64::from(direction.signum()) * self.step() * if fine { 0.1 } else { 1.0 };
        match self {
            Self::X => title.anchor[0] = value.clamp(0.0, 1.0),
            Self::Y => title.anchor[1] = value.clamp(0.0, 1.0),
            Self::Size => title.size = value.clamp(0.01, 1.0),
            Self::Red => title.color[0] = value.clamp(0.0, 1.0),
            Self::Green => title.color[1] = value.clamp(0.0, 1.0),
            Self::Blue => title.color[2] = value.clamp(0.0, 1.0),
            Self::Opacity => title.color[3] = value.clamp(0.0, 1.0),
            Self::FadeIn | Self::FadeOut => {
                let value =
                    MediaTime::from_seconds(value.clamp(0.0, title.span.duration().seconds()))
                        .unwrap_or(title.span.duration());
                match self {
                    Self::FadeIn => title.fade_in = value,
                    Self::FadeOut => title.fade_out = value,
                    _ => unreachable!(),
                }
            }
        }
    }
}

impl TitleClip {
    /// Returns zero outside the half-open title span, with independent overlapping fades inside it.
    pub fn opacity_at(&self, at: MediaTime) -> f64 {
        if !self.span.contains(at) {
            return 0.0;
        }
        let fade = |elapsed: MediaTime, duration: MediaTime| {
            if duration == MediaTime::ZERO {
                1.0
            } else {
                (elapsed.0 as f64 / duration.0 as f64).clamp(0.0, 1.0)
            }
        };
        self.color[3]
            * fade(at.saturating_sub(self.span.start), self.fade_in)
            * fade(self.span.end.saturating_sub(at), self.fade_out)
    }
}
