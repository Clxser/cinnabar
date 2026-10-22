//! Independent scalar camera channels retain their source phase through trims and ordinary undo edits.

use super::*;
use crate::effect::Curve;
use std::collections::BTreeMap;

macro_rules! channels {
    ($($kind:ident => ($label:literal, $get:expr, $set:expr)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum CameraChannel { $($kind),+ }
        impl CameraChannel {
            pub const ALL: &[Self] = &[$(Self::$kind),+];
            /// Native graph tabs and saved channels share one exhaustive registry.
            pub const fn label(self) -> &'static str { match self { $(Self::$kind => $label),+ } }
            /// Reads the authored scalar without wrapping rotations or changing the rig.
            pub fn value(self, pose: CameraPose) -> f64 { match self { $(Self::$kind => ($get)(pose)),+ } }
            /// Changes only one channel; pitch retains the valid projection poles.
            pub fn assign(self, pose: &mut CameraPose, value: f64) { match self { $(Self::$kind => ($set)(pose, value)),+ } }
        }
    }
}
channels! {
    X => ("X", |p: CameraPose| p.position[0], |p: &mut CameraPose, v| p.position[0] = v),
    Y => ("Y", |p: CameraPose| p.position[1], |p: &mut CameraPose, v| p.position[1] = v),
    Z => ("Z", |p: CameraPose| p.position[2], |p: &mut CameraPose, v| p.position[2] = v),
    Yaw => ("Yaw", |p: CameraPose| p.yaw, |p: &mut CameraPose, v| p.yaw = v),
    Pitch => ("Pitch", |p: CameraPose| p.pitch, |p: &mut CameraPose, v: f64| p.pitch = v.clamp(-90.0, 90.0)),
    Roll => ("Roll", |p: CameraPose| p.roll, |p: &mut CameraPose, v| p.roll = v),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraAnimation {
    #[serde(default)]
    pub curves: BTreeMap<CameraChannel, CameraChannelCurve>,
}

impl CameraAnimation {
    /// Legacy shots omit an empty animation owner.
    pub fn is_empty(&self) -> bool {
        self.curves.is_empty()
    }

    /// Curves retain support keys outside the visible trim but must always produce finite admitted values.
    pub(super) fn validate(&self, duration: MediaTime) -> Result<(), StudioError> {
        for (&channel, channel_curve) in &self.curves {
            channel_curve.offset.checked_add(duration)?;
            let curve = &channel_curve.curve;
            limit("camera channel keys", curve.keys.len(), MAX_CAMERA_KEYS)?;
            if curve.keys.is_empty()
                || curve.keys.windows(2).any(|p| p[0].at >= p[1].at)
                || curve.keys.iter().any(|k| {
                    !k.value.is_finite()
                        || k.at.0 > crate::time::MAX_MEDIA_MICROS
                        || (channel == CameraChannel::Pitch && k.value.abs() > 90.0)
                })
            {
                return Err(StudioError::Invalid(
                    "camera graph keys must be finite and ordered with valid pitch",
                ));
            }
        }
        Ok(())
    }

    /// Sampling uses no editor history, so seeks and out-of-order exported frames agree.
    pub(super) fn apply(&self, pose: &mut CameraPose, elapsed: MediaTime) {
        for (&channel, channel_curve) in &self.curves {
            let at = MediaTime(channel_curve.offset.0.saturating_add(elapsed.0));
            channel.assign(pose, channel_curve.curve.sample(at));
        }
    }
}

/// Each scalar channel owns its original phase, including support keys outside a retained trim.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraChannelCurve {
    pub curve: Curve,
    #[serde(default)]
    pub offset: MediaTime,
}
