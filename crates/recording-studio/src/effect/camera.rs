//! Applying an inspected view authors the underlying rig without baking transient shake or zoom twice.

use super::EffectSample;
use crate::{MediaTime, StudioError, camera::CameraPose};

impl EffectSample {
    /// Inverts camera effects at the selected key time, rejecting an unrepresentable base FOV or pitch.
    pub fn camera_base_pose(
        self,
        mut displayed: CameraPose,
        at: MediaTime,
    ) -> Result<CameraPose, StudioError> {
        displayed.validate()?;
        let offset = self.camera_pose(CameraPose::default(), at);
        displayed.yaw -= offset.yaw;
        displayed.pitch -= offset.pitch;
        displayed.fov += self.zoom_degrees;
        displayed.validate()?;
        Ok(displayed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applying_a_displayed_pose_does_not_bake_camera_effects_twice() {
        let effects = EffectSample {
            zoom_degrees: 15.0,
            shake_degrees: 3.0,
            ..EffectSample::default()
        };
        let at = MediaTime(123_456);
        let pose = CameraPose {
            yaw: 25.0,
            pitch: 10.0,
            roll: 8.0,
            ..CameraPose::default()
        };
        let displayed = effects.camera_pose(pose, at);
        let base = effects.camera_base_pose(displayed, at).unwrap();
        assert!((base.yaw - pose.yaw).abs() < 1e-10);
        assert!((base.pitch - pose.pitch).abs() < 1e-10);
        assert_eq!(base.fov, pose.fov);
        assert_eq!(base.position, pose.position);
        assert_eq!(base.roll, pose.roll);
        assert!(
            effects
                .camera_base_pose(CameraPose { fov: 178.0, ..pose }, at)
                .is_err()
        );
    }
}
