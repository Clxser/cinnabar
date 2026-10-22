//! Terminal authored rates compile on edits; recorded subjects remain current-time inputs.

use super::*;
use crate::effect::Curve;

const MAX_DERIVATIVE_STEP: MediaTime = MediaTime(1_000);
const FIXED_ACTOR: ActorPose = ActorPose {
    position: [0.0; 3],
    velocity: [0.0; 3],
    yaw: 0.0,
    pitch: 0.0,
    roll: 0.0,
    eye_offset: None,
};

#[derive(Clone, Copy, Default)]
pub(crate) struct TerminalMotion {
    linear: DVec3,
    /// Network yaw, pitch and roll rates resolve against the actual terminal view, never a dummy frame.
    angular: DVec3,
    lens: f64,
}

impl TerminalMotion {
    /// Reads three authored times and retained geometry, without requesting past or future recorded actors.
    pub(super) fn new(shot: &CameraShot, path: Option<&CompiledCameraPath>) -> Self {
        let prepared = path
            .is_none()
            .then(|| CompiledCameraPath::new(shot))
            .flatten();
        let path = path.or(prepared.as_ref());
        let mut motion = Self::default();
        let step = derivative_step(shot);
        let end = shot.span.end;
        let one = end.saturating_sub(step);
        let two = end
            .saturating_sub(MediaTime(step.0 * 2))
            .max(shot.span.start);
        let seconds = step.seconds();
        let derivative = |last: f64, first: f64, second: f64| {
            if two < one {
                2.0 * (last - first) / seconds - (last - second) / (2.0 * seconds)
            } else {
                (last - first) / seconds
            }
        };
        if let Some((last, first, second)) = base_path_pose(shot, path, end)
            .zip(base_path_pose(shot, path, one))
            .zip(base_path_pose(shot, path, two))
            .map(|((last, first), second)| (last, first, second))
        {
            motion.linear = DVec3::from_array(std::array::from_fn(|axis| {
                derivative(
                    last.position[axis],
                    first.position[axis],
                    second.position[axis],
                )
            }));
            motion.angular = DVec3::new(
                derivative(
                    0.0,
                    -wrapped_delta(last.yaw, first.yaw),
                    -wrapped_delta(last.yaw, second.yaw),
                ),
                derivative(last.pitch, first.pitch, second.pitch),
                derivative(
                    0.0,
                    -wrapped_delta(last.roll, first.roll),
                    -wrapped_delta(last.roll, second.roll),
                ),
            );
            motion.lens = derivative(last.fov, first.fov, second.fov);
            if let Some(path) = path {
                // A trimmed endpoint on an interior key retains this shot's left-hand velocity.
                motion.linear = DVec3::from_array(
                    path.velocity(MediaTime(end.0 - 1))
                        .unwrap_or(motion.linear.to_array()),
                );
            }
            let source = shot.path_offset.checked_add(shot.span.duration()).ok();
            if source
                .and_then(|at| shot.path_keys().iter().find(|key| key.at == at))
                .is_some_and(|key| key.easing == Easing::Hold)
            {
                motion.angular = DVec3::ZERO;
                if shot.fov_curve.is_none() {
                    motion.lens = 0.0;
                }
                if shot.path_timing == PathTiming::Keyed {
                    motion.linear = DVec3::ZERO;
                }
            }
        } else if let CameraMode::Orbit {
            radius,
            start_degrees,
            turns,
            ..
        } = shot.mode
        {
            let angle = (start_degrees + turns * 360.0).to_radians();
            let omega = turns * std::f64::consts::TAU / shot.span.duration().seconds();
            motion.linear = DVec3::new(
                radius * angle.cos() * omega,
                0.0,
                -radius * angle.sin() * omega,
            );
            motion.angular.x = -turns * 360.0 / shot.span.duration().seconds();
        }
        for (&channel, curve) in &shot.animation.curves {
            let source = MediaTime(curve.offset.0.saturating_add(shot.span.duration().0));
            let rate = curve_rate(&curve.curve, source);
            let target = match channel {
                CameraChannel::X => &mut motion.linear.x,
                CameraChannel::Y => &mut motion.linear.y,
                CameraChannel::Z => &mut motion.linear.z,
                CameraChannel::Yaw => &mut motion.angular.x,
                CameraChannel::Pitch => &mut motion.angular.y,
                CameraChannel::Roll => &mut motion.angular.z,
            };
            *target = if curve.additive { *target + rate } else { rate };
        }
        // Only the fixed actor's authored lens is sampled; no native eye or subject trajectory is invented.
        let last = authored_pose(shot, path, end);
        if let Some(fov) = shot.fov_override(end) {
            motion.lens = derivative(
                fov,
                shot.fov_override(one).unwrap_or(fov),
                shot.fov_override(two).unwrap_or(fov),
            );
            let source = shot.fov_offset.checked_add(shot.span.duration()).ok();
            if shot
                .fov_curve
                .as_ref()
                .zip(source)
                .is_some_and(|(curve, at)| {
                    curve
                        .keys
                        .iter()
                        .any(|key| key.at == at && key.easing == Easing::Hold)
                })
            {
                motion.lens = 0.0;
            }
        } else if !matches!(shot.mode, CameraMode::Path { .. })
            && let Some((last, first, second)) = last
                .zip(authored_pose(shot, path, one))
                .zip(authored_pose(shot, path, two))
                .map(|((last, first), second)| (last, first, second))
        {
            motion.lens = derivative(last.fov, first.fov, second.fov);
        }
        if matches!(
            shot.mode,
            CameraMode::Free { .. } | CameraMode::Path { .. } | CameraMode::Orbit { .. }
        ) && last.is_some_and(|pose| pose.pitch.abs() >= 90.0)
        {
            motion.angular.y = 0.0;
        }
        if !motion.linear.is_finite() {
            motion.linear = DVec3::ZERO;
        }
        if !motion.angular.is_finite() {
            motion.angular = DVec3::ZERO;
        }
        if !motion.lens.is_finite() {
            motion.lens = 0.0;
        }
        motion
    }

    /// Extends authored motion in the actual terminal frame, without integration history or actor prediction.
    pub(crate) fn extrapolated(self, terminal: CameraPose, elapsed: MediaTime) -> CameraPose {
        if elapsed == MediaTime::ZERO {
            return terminal;
        }
        let seconds = elapsed.seconds();
        let mut pose = terminal;
        let position = DVec3::from_array(terminal.position) + self.linear * seconds;
        if position.is_finite() {
            pose.position = position.to_array();
        }
        if self.lens != 0.0 {
            pose.fov = (terminal.fov + self.lens * seconds).clamp(
                CAMERA_FOV_RANGE.start.next_up(),
                CAMERA_FOV_RANGE.end.next_down(),
            );
        }
        if self.angular != DVec3::ZERO {
            let yaw = (180.0 - terminal.yaw.rem_euclid(360.0)).to_radians();
            let pitch = -terminal.pitch.to_radians();
            // A projection pole is an intentional clamp; do not infer motion through an unavailable unclamped pitch.
            let pitch_rate = if terminal.pitch.abs() >= 90.0 {
                0.0
            } else {
                self.angular.y
            };
            let angular = -DVec3::Y * self.angular.x.to_radians()
                - (DQuat::from_rotation_y(yaw) * DVec3::X) * pitch_rate.to_radians()
                + (DQuat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0) * DVec3::Z)
                    * self.angular.z.to_radians();
            if angular != DVec3::ZERO {
                pose = with_orientation(
                    pose,
                    (DQuat::from_scaled_axis(angular * seconds) * orientation(terminal))
                        .normalize(),
                );
            }
        }
        pose
    }
}

/// The original geometric sampler excludes independent scalar channels, preserving exact base-rail rates.
fn base_path_pose(
    shot: &CameraShot,
    path: Option<&CompiledCameraPath>,
    at: MediaTime,
) -> Option<CameraPose> {
    path.and_then(|path| path.sample(at))
        .or_else(|| shot.path_pose(at))
}

/// Fixed mathematical actor context carries no eye provenance and never reaches the replay publication.
fn authored_pose(
    shot: &CameraShot,
    path: Option<&CompiledCameraPath>,
    at: MediaTime,
) -> Option<CameraPose> {
    if let CameraMode::Player { fov, .. } = shot.mode {
        let mut pose = CameraPose {
            fov,
            ..Default::default()
        };
        shot.animation
            .apply(&mut pose, at.saturating_sub(shot.span.start));
        if let Some(fov) = shot.fov_override(at) {
            pose.fov = fov;
        }
        Some(pose)
    } else if at == shot.span.end {
        shot.sample_terminal(|_| Some(FIXED_ACTOR), path)
    } else {
        shot.sample_compiled(at, |_| Some(FIXED_ACTOR), path)
    }
}

/// Scalar channels retain their authored winding; the arriving interval supplies a deterministic left derivative.
fn curve_rate(curve: &Curve, source: MediaTime) -> f64 {
    let next = curve.keys.partition_point(|key| key.at < source);
    let Some((a, b)) = next
        .checked_sub(1)
        .and_then(|index| curve.keys.get(index).zip(curve.keys.get(next)))
    else {
        return 0.0;
    };
    let duration = b.at.saturating_sub(a.at).seconds();
    let amount = source.saturating_sub(a.at).seconds() / duration;
    (b.value - a.value) * b.easing.derivative(amount) / duration
}

/// Original rail angles use the same shortest component arcs as their authoritative sampler.
fn wrapped_delta(last: f64, previous: f64) -> f64 {
    (last.rem_euclid(360.0) - previous.rem_euclid(360.0) + 180.0).rem_euclid(360.0) - 180.0
}

/// Backward samples stay in the final authored interval, including each scalar channel's retained phase.
fn derivative_step(shot: &CameraShot) -> MediaTime {
    let mut available = shot.span.duration().0;
    if let Ok(source) = shot.path_offset.checked_add(shot.span.duration()) {
        available = available
            .min(interval_tail(shot.path_keys(), source, |key| key.at).unwrap_or(available));
    }
    if let Some(curve) = &shot.fov_curve
        && let Ok(source) = shot.fov_offset.checked_add(shot.span.duration())
    {
        available =
            available.min(interval_tail(&curve.keys, source, |key| key.at).unwrap_or(available));
    }
    for curve in shot.animation.curves.values() {
        if let Ok(source) = curve.offset.checked_add(shot.span.duration()) {
            available = available
                .min(interval_tail(&curve.curve.keys, source, |key| key.at).unwrap_or(available));
        }
    }
    MediaTime((available / 2).clamp(1, MAX_DERIVATIVE_STEP.0))
}

/// Binary lookup avoids materializing copied key times during compilation.
fn interval_tail<T>(keys: &[T], source: MediaTime, at: impl Fn(&T) -> MediaTime) -> Option<u64> {
    let index = keys
        .partition_point(|key| at(key) < source)
        .checked_sub(1)?;
    Some(source.0 - at(&keys[index]).0)
}

#[cfg(test)]
mod tests;
