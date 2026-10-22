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
}

impl Look {
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
