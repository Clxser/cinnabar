//! Terminal authored derivatives are compiled on edits; recorded subject motion stays current-time data.

use super::*;

const MAX_DERIVATIVE_STEP: MediaTime = MediaTime(1_000);

#[derive(Clone, Copy, Default)]
pub(crate) struct TerminalMotion {
    linear: DVec3,
    angular: DVec3,
    lens: f64,
}

impl TerminalMotion {
    /// Reads at most three authored times and the retained distance cache, without querying an actor.
    pub(super) fn new(shot: &CameraShot, path: Option<&CompiledCameraPath>) -> Self {
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
        if matches!(shot.mode, CameraMode::Free { .. } | CameraMode::Path { .. })
            && let Some((last, first, second)) = shot
                .sample_terminal(|_| None, path)
                .zip(shot.sample_compiled(one, |_| None, path))
                .zip(shot.sample_compiled(two, |_| None, path))
                .map(|((last, first), second)| (last, first, second))
        {
            motion.linear = DVec3::from_array(std::array::from_fn(|axis| {
                derivative(
                    last.position[axis],
                    first.position[axis],
                    second.position[axis],
                )
            }));
            let delta = |previous| {
                let mut rotation = orientation(last) * orientation(previous).conjugate();
                if rotation.w < 0.0 {
                    rotation = -rotation;
                }
                rotation.to_scaled_axis()
            };
            motion.angular = if two < one {
                2.0 * delta(first) / seconds - delta(second) / (2.0 * seconds)
            } else {
                delta(first) / seconds
            };
            motion.lens = derivative(last.fov, first.fov, second.fov);
            if let Some(path) = path {
                // The left limit belongs to this shot even when a trim ends on an interior key.
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
            motion.angular = DVec3::Y * omega;
        }
        if let Some(last) = shot.fov_override(end) {
            motion.lens = derivative(
                last,
                shot.fov_override(one).unwrap_or(last),
                shot.fov_override(two).unwrap_or(last),
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

    /// Extends only authored terminal motion; no future actor/world samples or integration history exist.
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
            pose = with_orientation(
                pose,
                (DQuat::from_scaled_axis(self.angular * seconds) * orientation(terminal))
                    .normalize(),
            );
        }
        pose
    }
}

/// Backward samples stay in the final authored interval where microsecond resolution permits it.
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
