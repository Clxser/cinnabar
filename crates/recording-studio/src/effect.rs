//! Small composable effects evaluated from project time, including editable automation output.

use serde::{Deserialize, Serialize};

use crate::{Id, MediaTime, Span, StudioError, error::limit};

pub const MAX_CURVE_KEYS: usize = 4096;
pub const MAX_EFFECT_VALUE: f64 = 1_000_000.0;

mod edit;
pub mod preset;

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
    /// Native key editors use the same exhaustive names and cycle as saved curve interpolation.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Smooth => "Smooth",
            Self::EaseIn => "Ease in",
            Self::EaseOut => "Ease out",
            Self::Hold => "Hold",
        }
    }

    /// Adding an interpolation mode requires choosing its place in the native cycle.
    pub const fn next(self) -> Self {
        match self {
            Self::Linear => Self::Smooth,
            Self::Smooth => Self::EaseIn,
            Self::EaseIn => Self::EaseOut,
            Self::EaseOut => Self::Hold,
            Self::Hold => Self::Linear,
        }
    }

    /// Maps bounded linear progress to a deterministic transition with exact endpoints.
    pub fn sample(self, value: f64) -> f64 {
        let t = value.clamp(0.0, 1.0);
        if self == Self::Hold {
            return if t >= 1.0 { 1.0 } else { 0.0 };
        }
        self.polynomial()
            .into_iter()
            .rev()
            .fold(0.0, |value, coefficient| value * t + coefficient)
            .clamp(0.0, 1.0)
    }

    /// Analytic curve area derives from the same polynomial as sampling, keeping speed-map integrals in sync.
    pub fn integral(self, value: f64) -> f64 {
        let t = value.clamp(0.0, 1.0);
        self.polynomial()
            .into_iter()
            .enumerate()
            .rev()
            .fold(0.0, |value, (power, coefficient)| {
                value * t + coefficient / (power + 1) as f64
            })
            * t
    }

    /// One coefficient definition owns both curve evaluation and its source-clock integral.
    fn polynomial(self) -> [f64; 4] {
        match self {
            Self::Linear => [0.0, 1.0, 0.0, 0.0],
            Self::Smooth => [0.0, 0.0, 3.0, -2.0],
            Self::EaseIn => [0.0, 0.0, 0.0, 1.0],
            Self::EaseOut => [0.0, 3.0, -3.0, 1.0],
            Self::Hold => [0.0; 4],
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

macro_rules! effects {
    ($($kind:ident => ($label:literal, $initial:literal, $step:literal, $min:literal, $max:literal)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum EffectKind { $($kind),+ }
        impl EffectKind {
            pub const ALL: &[Self] = &[$(Self::$kind),+];
            pub const fn label(self) -> &'static str { match self { $(Self::$kind => $label),+ } }
            pub const fn initial(self) -> f64 { match self { $(Self::$kind => $initial),+ } }
            pub const fn step(self) -> f64 { match self { $(Self::$kind => $step),+ } }
            pub fn constrain(self, value: f64) -> f64 { match self { $(Self::$kind => value.clamp($min, $max)),+ } }
        }
    };
}
effects! {
    Zoom => ("Zoom", 8.0, 1.0, -90.0, 90.0),
    Shake => ("Shake", 0.5, 0.1, 0.0, 8.0),
    Exposure => ("Exposure", 0.5, 0.1, -8.0, 8.0),
    Saturation => ("Saturation", 0.25, 0.1, -1.0, 3.0),
    Vignette => ("Vignette", 0.3, 0.05, 0.0, 1.0),
    Bloom => ("Bloom", 0.3, 0.1, 0.0, 4.0),
    Flash => ("Flash", 0.5, 0.05, 0.0, 1.0),
    Letterbox => ("Letterbox", 0.1, 0.01, 0.0, 0.4),
    Fade => ("Fade", 0.5, 0.05, 0.0, 1.0),
    Contrast => ("Contrast", 0.2, 0.05, -1.0, 1.0),
    Warmth => ("Warmth", 0.2, 0.05, -1.0, 1.0),
    Tint => ("Tint", 0.15, 0.05, -1.0, 1.0),
    Grain => ("Film grain", 0.15, 0.025, 0.0, 1.0),
    Chromatic => ("RGB split", 0.003, 0.001, 0.0, 0.03),
    SoftFocus => ("Soft focus", 0.25, 0.05, 0.0, 1.0),
    Sharpen => ("Sharpen", 0.25, 0.05, 0.0, 2.0),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationOrigin {
    pub source: Id,
    pub event_sequence: u64,
    pub template_revision: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_time: Option<MediaTime>,
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
    pub contrast: f64,
    pub warmth: f64,
    pub tint: f64,
    pub grain: f64,
    pub chromatic: f64,
    pub soft_focus: f64,
    pub sharpen: f64,
}

impl EffectSample {
    /// Render edges can retain an unchanged frame without allocating another composition target.
    pub fn neutral(self) -> bool {
        self == Self::default()
    }

    /// Native replay already applies these two effects to its world camera before raster composition.
    pub fn without_camera(mut self) -> Self {
        self.zoom_degrees = 0.0;
        self.shake_degrees = 0.0;
        self
    }

    /// Ordinary video uses an affine image crop/translation, with a shared virtual FOV for angle units.
    /// This cannot reveal pixels outside the recording or synthesize the parallax of a 3D replay.
    pub fn image_motion(self, at: MediaTime) -> [f32; 3] {
        let base = crate::camera::CameraPose::default();
        let pose = self.camera_pose(base, at);
        let tangent = (base.fov.to_radians() * 0.5).tan();
        [
            (tangent / (pose.fov.to_radians() * 0.5).tan()) as f32,
            (-pose.yaw.to_radians().tan() / (2.0 * tangent)) as f32,
            (pose.pitch.to_radians().tan() / (2.0 * tangent)) as f32,
        ]
    }

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
            EffectKind::Contrast => &mut self.contrast,
            EffectKind::Warmth => &mut self.warmth,
            EffectKind::Tint => &mut self.tint,
            EffectKind::Grain => &mut self.grain,
            EffectKind::Chromatic => &mut self.chromatic,
            EffectKind::SoftFocus => &mut self.soft_focus,
            EffectKind::Sharpen => &mut self.sharpen,
        };
        *slot += value;
    }

    /// Bounds the final sum once, after all active tracks and transition contributions are known.
    pub fn constrain(&mut self) {
        self.zoom_degrees = EffectKind::Zoom.constrain(self.zoom_degrees);
        self.shake_degrees = EffectKind::Shake.constrain(self.shake_degrees);
        self.exposure_stops = EffectKind::Exposure.constrain(self.exposure_stops);
        self.saturation_delta = EffectKind::Saturation.constrain(self.saturation_delta);
        self.vignette = EffectKind::Vignette.constrain(self.vignette);
        self.bloom = EffectKind::Bloom.constrain(self.bloom);
        self.flash = EffectKind::Flash.constrain(self.flash);
        self.letterbox = EffectKind::Letterbox.constrain(self.letterbox);
        self.fade = EffectKind::Fade.constrain(self.fade);
        self.contrast = EffectKind::Contrast.constrain(self.contrast);
        self.warmth = EffectKind::Warmth.constrain(self.warmth);
        self.tint = EffectKind::Tint.constrain(self.tint);
        self.grain = EffectKind::Grain.constrain(self.grain);
        self.chromatic = EffectKind::Chromatic.constrain(self.chromatic);
        self.soft_focus = EffectKind::SoftFocus.constrain(self.soft_focus);
        self.sharpen = EffectKind::Sharpen.constrain(self.sharpen);
    }
}
