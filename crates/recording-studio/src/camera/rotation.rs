//! Authored Euler rotations retain position, lens and rail geometry without sampling a new pose key.

use super::*;

macro_rules! axes {
    ($($axis:ident => ($label:literal, $basis:literal, $parameter:ident)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum CameraRotationAxis { $($axis),+ }
        impl CameraRotationAxis {
            pub const ALL: [Self; 3] = [$(Self::$axis),+];
            /// A named handle matches the existing numeric inspector parameter.
            pub const fn label(self) -> &'static str { match self { $(Self::$axis => $label),+ } }
            /// The renderer's YXZ Euler basis, rather than an arbitrary world/local quaternion blend.
            pub const fn basis(self) -> &'static str { match self { $(Self::$axis => $basis),+ } }
            /// One component owns each authored rotation axis.
            pub const fn parameter(self) -> CameraParameter { match self { $(Self::$axis => CameraParameter::$parameter),+ } }
        }
    };
}
axes! {
    Yaw => ("Yaw", "World Y", Yaw),
    Pitch => ("Pitch", "Yaw-local X", Pitch),
    Roll => ("Roll", "View forward", Roll),
}

impl CameraRotationAxis {
    /// Returns the exact authored component; angles remain unwrapped throughout a drag.
    pub fn value(self, pose: CameraPose) -> f64 {
        match self {
            Self::Yaw => pose.yaw,
            Self::Pitch => pose.pitch,
            Self::Roll => pose.roll,
        }
    }

    /// Applies only this Euler component; pitch stops at its admitted poles.
    fn assign(self, pose: &mut CameraPose, value: f64) {
        match self {
            Self::Yaw => pose.yaw = value,
            Self::Pitch => pose.pitch = value.clamp(-90.0, 90.0),
            Self::Roll => pose.roll = value,
        }
    }
}

impl CameraShot {
    /// Rotation edits own a Free pose or an exact visible Path key, including independent global rails.
    pub fn rotation_pose(&self, at: MediaTime) -> Result<CameraPose, StudioError> {
        if at < self.span.start || at > self.span.end {
            return Err(StudioError::Invalid(
                "Move inside the camera shot before rotating.",
            ));
        }
        match &self.mode {
            CameraMode::Free { pose } => Ok(*pose),
            CameraMode::Path { .. } => {
                self.key_at(at)
                    .map(|key| key.pose)
                    .ok_or(StudioError::Invalid(
                        "Select an existing path key on the timeline before rotating.",
                    ))
            }
            _ => Err(StudioError::Invalid(
                "Rotation handles edit Free and Path cameras. Use this rig's numeric controls.",
            )),
        }
    }

    /// Replaces one exact orientation component without rebuilding or changing its positional rail.
    pub fn with_rotation(
        &self,
        at: MediaTime,
        axis: CameraRotationAxis,
        value: f64,
    ) -> Result<Self, StudioError> {
        self.rotation_pose(at)?;
        if !value.is_finite() {
            return Err(StudioError::Invalid("Camera rotation must be finite."));
        }
        let mut next = self.clone();
        match &mut next.mode {
            CameraMode::Free { pose } => axis.assign(pose, value),
            CameraMode::Path { keys, .. } => {
                let offset = self.key_at(at).expect("exact key admitted").at;
                let index = keys
                    .binary_search_by_key(&offset, |key| key.at)
                    .expect("exact key retained");
                axis.assign(&mut keys[index].pose, value);
            }
            _ => unreachable!("rotation owner admitted"),
        }
        next.validate()?;
        Ok(next)
    }
}

/// Frozen screen-ring geometry makes pointer capture independent of new previews and layout changes.
#[derive(Clone, Copy, Debug)]
pub struct CameraRotationDrag {
    axis: CameraRotationAxis,
    origin: f64,
    center: [f64; 2],
    radius: [f64; 2],
    last: f64,
    degrees: f64,
}

impl CameraRotationDrag {
    /// Binds the initial authored angle and ring ellipse once, never the currently edited preview.
    pub fn new(
        pose: CameraPose,
        axis: CameraRotationAxis,
        bounds: [i32; 4],
        down: [f32; 2],
    ) -> Option<Self> {
        pose.validate().ok()?;
        if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
            return None;
        }
        let mut drag = Self {
            axis,
            origin: axis.value(pose),
            center: [
                (f64::from(bounds[0]) + f64::from(bounds[2])) * 0.5,
                (f64::from(bounds[1]) + f64::from(bounds[3])) * 0.5,
            ],
            radius: [
                (f64::from(bounds[2]) - f64::from(bounds[0])) * 0.5,
                (f64::from(bounds[3]) - f64::from(bounds[1])) * 0.5,
            ],
            last: 0.0,
            degrees: 0.0,
        };
        drag.last = drag.angle(down)?;
        Some(drag)
    }

    /// Successive signed arcs unwrap ±180 crossings; crossing the center leaves the last valid arc.
    /// One pointer sample cannot distinguish turns larger than 180 degrees, so callers deliver motion samples.
    pub fn sample(&mut self, point: [f32; 2]) -> Option<f64> {
        let next = self.angle(point)?;
        self.degrees += (next - self.last + 180.0).rem_euclid(360.0) - 180.0;
        self.last = next;
        let mut value = self.origin + self.degrees;
        if self.axis == CameraRotationAxis::Pitch {
            value = value.clamp(-90.0, 90.0);
            self.degrees = value - self.origin;
        }
        value.is_finite().then_some(value)
    }

    /// Ellipse normalization gives each visible ring the same angular input contract.
    fn angle(&self, point: [f32; 2]) -> Option<f64> {
        let [x, y] = std::array::from_fn(|axis| {
            (f64::from(point[axis]) - self.center[axis]) / self.radius[axis]
        });
        (x.is_finite() && y.is_finite() && x * x + y * y >= 0.01).then(|| y.atan2(x).to_degrees())
    }
}

#[cfg(test)]
mod tests;
