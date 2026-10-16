//! Small composable effects evaluated from project time, including editable automation output.

use serde::{Deserialize, Serialize};

use crate::{Id, MediaTime, Span, StudioError, error::limit};

pub const MAX_CURVE_KEYS: usize = 4096;
pub const MAX_EFFECT_VALUE: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    Linear,
    #[default]
    Smooth,
    EaseIn,
    EaseOut,
    Hold,
}

impl Easing {
    /// Maps bounded linear progress to a deterministic transition with exact endpoints.
    pub fn sample(self, value: f64) -> f64 {
        let t = value.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::Smooth => t * t * (3.0 - 2.0 * t),
            Self::EaseIn => t * t * t,
            Self::EaseOut => 1.0 - (1.0 - t).powi(3),
            Self::Hold => {
                if t >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurveKey {
    pub at: MediaTime,
    pub value: f64,
    pub easing: Easing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Curve {
    pub keys: Vec<CurveKey>,
}

impl Curve {
    /// Makes a constant value without introducing frame-dependent state.
    pub fn constant(value: f64) -> Self {
        Self {
            keys: vec![CurveKey {
                at: MediaTime::ZERO,
                value,
                easing: Easing::Linear,
            }],
        }
    }

    /// Admits ordered relative-time keys within the owning clip's duration.
    pub fn validate(&self, duration: MediaTime) -> Result<(), StudioError> {
        limit("effect curve keys", self.keys.len(), MAX_CURVE_KEYS)?;
        if self.keys.is_empty()
            || self.keys.iter().any(|key| {
                !key.value.is_finite() || key.value.abs() > MAX_EFFECT_VALUE || key.at > duration
            })
            || self.keys.windows(2).any(|pair| pair[0].at >= pair[1].at)
        {
            return Err(StudioError::Invalid(
                "effect keys must be finite, ordered and within their clip",
            ));
        }
        Ok(())
    }

    /// Binary search keeps per-frame work independent of the project's total key count.
    pub fn sample(&self, at: MediaTime) -> f64 {
        let next = self.keys.partition_point(|key| key.at <= at);
        if next == 0 {
            return self.keys.first().map_or(0.0, |key| key.value);
        }
        let a = self.keys[next - 1];
        let Some(b) = self.keys.get(next) else {
            return a.value;
        };
        let t = b
            .easing
            .sample((at.0 - a.at.0) as f64 / (b.at.0 - a.at.0) as f64);
        a.value + (b.value - a.value) * t
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    Zoom,
    Shake,
    Exposure,
    Saturation,
    Vignette,
    Bloom,
    Flash,
    Letterbox,
    Fade,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationOrigin {
    pub source: Id,
    pub event_sequence: u64,
    pub template_revision: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectClip {
    pub id: Id,
    pub span: Span,
    pub kind: EffectKind,
    pub amount: Curve,
    pub enabled: bool,
    pub origin: Option<AutomationOrigin>,
}

impl EffectClip {
    /// Zero outside the half-open clip makes effect boundaries and cuts predictable.
    pub fn sample(&self, time: MediaTime) -> f64 {
        if !self.enabled || !self.span.contains(time) {
            return 0.0;
        }
        self.amount.sample(time.saturating_sub(self.span.start))
    }
}

/// Values consumed by one preview or exported frame after summing active effect tracks.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EffectSample {
    pub zoom_degrees: f64,
    pub shake_degrees: f64,
    pub exposure_stops: f64,
    pub saturation_delta: f64,
    pub vignette: f64,
    pub bloom: f64,
    pub flash: f64,
    pub letterbox: f64,
    pub fade: f64,
}

impl EffectSample {
    /// Applies transient camera effects after the saved rig pose, identically for recorded and custom views.
    pub fn camera_pose(
        self,
        mut pose: crate::camera::CameraPose,
        at: MediaTime,
    ) -> crate::camera::CameraPose {
        pose.fov = (pose.fov - self.zoom_degrees).clamp(1.0, 178.0);
        let seconds = at.seconds();
        pose.yaw += self.shake_degrees * (seconds * 53.0).sin() * (seconds * 19.0).cos();
        pose.pitch = (pose.pitch + self.shake_degrees * (seconds * 47.0).sin()).clamp(-90.0, 90.0);
        pose
    }

    /// Accumulates authored values without making opposing effects depend on track iteration order.
    pub fn add(&mut self, kind: EffectKind, value: f64) {
        let slot = match kind {
            EffectKind::Zoom => &mut self.zoom_degrees,
            EffectKind::Shake => &mut self.shake_degrees,
            EffectKind::Exposure => &mut self.exposure_stops,
            EffectKind::Saturation => &mut self.saturation_delta,
            EffectKind::Vignette => &mut self.vignette,
            EffectKind::Bloom => &mut self.bloom,
            EffectKind::Flash => &mut self.flash,
            EffectKind::Letterbox => &mut self.letterbox,
            EffectKind::Fade => &mut self.fade,
        };
        *slot += value;
    }

    /// Bounds the final sum once, after all active tracks and transition contributions are known.
    pub fn constrain(&mut self) {
        self.zoom_degrees = self.zoom_degrees.clamp(-90.0, 90.0);
        self.shake_degrees = self.shake_degrees.clamp(0.0, 8.0);
        self.exposure_stops = self.exposure_stops.clamp(-8.0, 8.0);
        self.saturation_delta = self.saturation_delta.clamp(-1.0, 3.0);
        self.vignette = self.vignette.clamp(0.0, 1.0);
        self.bloom = self.bloom.clamp(0.0, 4.0);
        self.flash = self.flash.clamp(0.0, 1.0);
        self.letterbox = self.letterbox.clamp(0.0, 0.4);
        self.fade = self.fade.clamp(0.0, 1.0);
    }
}
