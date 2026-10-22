//! Authored looks remain ordinary editable effect clips rather than opaque baked filters.

use super::*;

macro_rules! looks {
    ($($kind:ident => ($label:literal, $pulse:literal, [$(($effect:ident, $amount:literal)),+ $(,)?])),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum Look { $($kind),+ }
        impl Look {
            pub const ALL: &[Self] = &[$(Self::$kind),+];
            pub const fn label(self) -> &'static str { match self { $(Self::$kind => $label),+ } }
            fn recipe(self) -> (bool, &'static [(EffectKind, f64)]) {
                match self { $(Self::$kind => ($pulse, &[$((EffectKind::$effect, $amount)),+]),)+ }
            }
        }
    };
}

looks! {
    Natural => ("Natural pop", false, [(Contrast, 0.12), (Saturation, 0.08), (Sharpen, 0.15)]),
    Impact => ("Impact", true, [(Zoom, 5.0), (Contrast, 0.2), (Chromatic, 0.003), (Flash, 0.08)]),
    Cinema => ("Cinema", false, [(Contrast, 0.18), (Warmth, 0.08), (Vignette, 0.18), (Letterbox, 0.08), (Grain, 0.025)]),
    Vintage => ("Vintage", false, [(Saturation, -0.35), (Warmth, 0.3), (Grain, 0.18), (Vignette, 0.25)]),
    Dream => ("Dream", false, [(SoftFocus, 0.3), (Bloom, 0.25), (Warmth, -0.1)]),
    Monochrome => ("Monochrome", false, [(Saturation, -1.0), (Contrast, 0.2), (Grain, 0.08)]),
    GoldenHour => ("Golden hour", false, [(Warmth, 0.22), (Contrast, 0.10), (Bloom, 0.10), (Vignette, 0.10)]),
    Nightfall => ("Nightfall", false, [(Warmth, -0.18), (Saturation, -0.12), (Contrast, 0.14), (Grain, 0.02)]),
    Crisp => ("Crisp detail", false, [(Sharpen, 0.25), (Contrast, 0.10), (Saturation, -0.04)]),
    GentleImpact => ("Soft impact", true, [(Zoom, 2.0), (Contrast, 0.08), (Bloom, 0.06)]),
}

impl Look {
    /// Impact looks remain short accents; grading looks can cover a whole selected clip.
    pub fn pulse(self) -> bool {
        self.recipe().0
    }

    /// Default insertion duration is owned by the preset rather than duplicated in menu code.
    pub fn duration(self) -> MediaTime {
        MediaTime(if self.pulse() { 600_000 } else { 3_000_000 })
    }

    /// Scales a reusable look while retaining editable keys, neutral boundaries and valid effect limits.
    pub fn clips_scaled(
        self,
        first: Id,
        span: Span,
        strength: f64,
    ) -> Result<Vec<EffectClip>, StudioError> {
        if !strength.is_finite() || !(0.0..=2.0).contains(&strength) {
            return Err(StudioError::Invalid(
                "look strength must be between zero and two",
            ));
        }
        let mut clips = self.clips(first, span)?;
        for clip in &mut clips {
            for key in &mut clip.amount.keys {
                key.value = clip.kind.constrain(key.value * strength);
            }
        }
        Ok(clips)
    }

    /// Constructs deterministic, independently editable envelopes with neutral boundaries.
    pub fn clips(self, first: Id, span: Span) -> Result<Vec<EffectClip>, StudioError> {
        span.validate()?;
        let (pulse, recipe) = self.recipe();
        let duration = span.duration().0;
        if duration < 4 {
            return Err(StudioError::Invalid(
                "a look needs at least four media microseconds",
            ));
        }
        recipe
            .iter()
            .enumerate()
            .map(|(index, &(kind, value))| {
                let id = first
                    .0
                    .checked_add(index as u64)
                    .ok_or(StudioError::IdentifiersExhausted)?;
                let mut keys = vec![
                    CurveKey {
                        at: MediaTime::ZERO,
                        value: 0.0,
                        easing: Easing::Linear,
                    },
                    CurveKey {
                        at: MediaTime((duration / if pulse { 4 } else { 8 }).max(1)),
                        value,
                        easing: Easing::Smooth,
                    },
                ];
                if !pulse {
                    keys.push(CurveKey {
                        at: MediaTime(duration - (duration / 8).max(1)),
                        value,
                        easing: Easing::Linear,
                    });
                }
                keys.push(CurveKey {
                    at: span.duration(),
                    value: 0.0,
                    easing: Easing::Smooth,
                });
                Ok(EffectClip {
                    id: Id(id),
                    span,
                    kind,
                    amount: Curve { keys },
                    enabled: true,
                    origin: None,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reusable_looks_keep_their_timing_neutral_boundaries_and_independent_editable_curves() {
        let span = Span::new(MediaTime(20_000_000), MediaTime(40_000_000)).unwrap();
        for &look in Look::ALL {
            let normal = look.clips(Id(1), span).unwrap();
            let stronger = look.clips_scaled(Id(1), span, 1.5).unwrap();
            assert_eq!(normal.len(), stronger.len());
            for (a, b) in normal.iter().zip(&stronger) {
                assert_eq!(a.id, b.id);
                assert_eq!(a.kind, b.kind);
                assert_eq!(b.span, span);
                b.amount.validate(span.duration()).unwrap();
                assert_eq!(b.amount.sample(MediaTime::ZERO), 0.0);
                assert_eq!(b.amount.sample(span.duration()), 0.0);
                let kind = a.kind;
                for (first, second) in a.amount.keys.iter().zip(&b.amount.keys) {
                    assert_eq!(first.at, second.at);
                    assert_eq!(second.value, kind.constrain(first.value * 1.5));
                }
            }
        }
        assert!(Look::Cinema.clips_scaled(Id(1), span, f64::NAN).is_err());
    }
}
