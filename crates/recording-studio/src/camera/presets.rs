//! Cinematic starting points produce ordinary editable shots from explicit recorded framing.

use super::*;

macro_rules! presets {
    ($($kind:ident => ($label:literal, $subject:literal)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum CameraPreset { $($kind),+ }
        impl CameraPreset {
            pub const ALL: &'static [Self] = &[$(Self::$kind),+];
            /// UI labels and availability share this registry with suggestion generation.
            pub const fn label(self) -> &'static str {
                match self { $(Self::$kind => $label),+ }
            }
            /// Subject-dependent proposals never synthesize a missing recorded identity.
            pub const fn needs_subject(self) -> bool {
                match self { $(Self::$kind => $subject),+ }
            }
        }
    }
}

presets! {
    PushIn => ("Push in", false),
    PullBack => ("Pull back", false),
    Arc => ("Arc reveal", false),
    OverShoulder => ("Over shoulder", true),
    TopDown => ("Top down", true),
}

impl CameraShot {
    /// A preset is a starting shot with editable keys/rig parameters, not an opaque procedural effect.
    pub fn cinematic_preset(
        id: Id,
        span: Span,
        preset: CameraPreset,
        pose: CameraPose,
        target: Option<&CameraTarget>,
    ) -> Result<Self, StudioError> {
        span.validate()?;
        pose.validate()?;
        if preset.needs_subject() && target.is_none() {
            return Err(StudioError::Invalid(
                "choose a recorded subject for this camera preset",
            ));
        }
        let center = target
            .map(|target| target.pose.centre())
            .or_else(|| pose.orbit_anchor(8.0))
            .ok_or(StudioError::Invalid(
                "the inspected camera has no valid forward direction",
            ))?;
        let mut shot = match preset {
            CameraPreset::OverShoulder => {
                let target = target.expect("subject admitted");
                let (sin, cos) = target.pose.yaw.to_radians().sin_cos();
                let center = DVec3::from_array(center);
                let shoulder =
                    center + DVec3::new(1.2 * cos + 3.5 * sin, 0.5, 1.2 * sin - 3.5 * cos);
                Self::from_preview(
                    id,
                    span,
                    CameraRigKind::Follow,
                    CameraPose {
                        position: shoulder.to_array(),
                        roll: 0.0,
                        fov: 55.0,
                        ..pose
                    }
                    .looking_at(center.to_array()),
                    Some(target),
                )?
            }
            CameraPreset::TopDown => {
                let position = [center[0], center[1] + 10.0, center[2] - 0.01];
                Self::from_preview(
                    id,
                    span,
                    CameraRigKind::Track,
                    CameraPose {
                        position,
                        roll: 0.0,
                        fov: 55.0,
                        ..pose
                    }
                    .looking_at(center),
                    target,
                )?
            }
            CameraPreset::Arc if target.is_some() => {
                let delta = DVec3::from_array(pose.position) - DVec3::from_array(center);
                let pose = if delta.x.hypot(delta.z) < 0.1 {
                    CameraPose {
                        position: [center[0] + 6.0, center[1] + 3.0, center[2] + 6.0],
                        ..pose
                    }
                    .looking_at(center)
                } else {
                    pose
                };
                Self::from_preview(id, span, CameraRigKind::Orbit, pose, target)?
            }
            CameraPreset::Arc => {
                if span.duration().0 < 2 {
                    return Err(StudioError::Invalid(
                        "an arc preset needs at least two camera time steps",
                    ));
                }
                let middle = pose
                    .orbited(center, [25.0, 0.0])
                    .ok_or(StudioError::Invalid(
                        "the arc pivot cannot form a camera orbit",
                    ))?;
                let end = pose
                    .orbited(center, [50.0, 0.0])
                    .ok_or(StudioError::Invalid(
                        "the arc pivot cannot form a camera orbit",
                    ))?;
                let mut shot = Self::from_preview(id, span, CameraRigKind::Path, pose, None)?;
                shot.mode = CameraMode::Path {
                    keys: vec![
                        key(MediaTime::ZERO, pose),
                        key(MediaTime(span.duration().0 / 2), middle),
                        key(span.duration(), end),
                    ],
                    smooth_position: true,
                };
                shot.path_timing = PathTiming::Distance;
                shot
            }
            CameraPreset::PushIn | CameraPreset::PullBack => {
                let destination = DVec3::from_array(center);
                let start = DVec3::from_array(pose.position);
                let direction =
                    (destination - start)
                        .try_normalize()
                        .ok_or(StudioError::Invalid(
                            "move away from the subject before creating a dolly",
                        ))?;
                let distance = (destination.distance(start) * 0.4).min(6.0);
                let sign = if preset == CameraPreset::PushIn {
                    1.0
                } else {
                    -1.0
                };
                let end = CameraPose {
                    position: (start + direction * distance * sign).to_array(),
                    ..pose
                };
                let mut shot = Self::from_preview(id, span, CameraRigKind::Path, pose, None)?;
                shot.mode = CameraMode::Path {
                    keys: vec![key(MediaTime::ZERO, pose), key(span.duration(), end)],
                    smooth_position: false,
                };
                shot.path_timing = PathTiming::Distance;
                shot
            }
        };
        shot.name = preset.label().into();
        shot.validate()?;
        Ok(shot)
    }
}

/// Preset keys keep an explicit orientation easing which the user may edit independently of travel.
fn key(at: MediaTime, pose: CameraPose) -> CameraKey {
    CameraKey {
        at,
        pose,
        easing: Easing::Smooth,
        handles: None,
    }
}

#[cfg(test)]
mod tests;
