//! The received rig stays authoritative; scalar graph offsets never bake a rail into approximate keys.

use super::*;
use crate::camera::{CameraMode, CameraShot, CompiledCameraPath};

pub(super) struct BaseRig {
    pub shot: CameraShot,
    pub path: Option<Arc<CompiledCameraPath>>,
}

impl BaseRig {
    /// One channel-free rig retains exact positional quadrature across animation-only document edits.
    pub fn new(shot: &CameraShot, previous: Option<&Arc<Self>>) -> Arc<Self> {
        let mut shot = shot.clone();
        shot.animation.curves.clear();
        if let Some(previous) = previous.filter(|old| old.shot == shot) {
            return Arc::clone(previous);
        }
        let path =
            CompiledCameraPath::new_reusing(&shot, previous.and_then(|old| old.path.as_deref()))
                .map(Arc::new);
        Arc::new(Self { shot, path })
    }

    /// Only Free and Path can be sampled without historical subject data on the editor thread.
    pub fn subject_relative(&self) -> bool {
        !matches!(
            self.shot.mode,
            CameraMode::Free { .. } | CameraMode::Path { .. }
        )
    }

    /// Inclusive authoring boundaries use the same cached rail and orientation clock as runtime playback.
    pub fn value(&self, channel: CameraChannel, at: MediaTime) -> f64 {
        let pose = match &self.shot.mode {
            CameraMode::Free { pose } => *pose,
            CameraMode::Path { .. } => self
                .path
                .as_ref()
                .and_then(|path| path.sample(at))
                .or_else(|| self.shot.path_pose(at))
                .expect("admitted inclusive path sample"),
            _ => unreachable!("subject-relative graphs never fabricate historical actor poses"),
        };
        channel.value(pose)
    }

    /// Clipped control hulls cover positional overshoots; orientation bounds include shortest-arc support.
    pub fn bounds(&self, channel: CameraChannel) -> [f64; 2] {
        let span = self.shot.span;
        let (a, b) = (
            self.value(channel, span.start),
            self.value(channel, span.end),
        );
        let mut bounds = [a.min(b), a.max(b)];
        let axis = match channel {
            CameraChannel::X => Some(0),
            CameraChannel::Y => Some(1),
            CameraChannel::Z => Some(2),
            CameraChannel::Yaw | CameraChannel::Pitch | CameraChannel::Roll => None,
        };
        if let Some(range) =
            axis.and_then(|axis| self.shot.coordinate_bounds(self.path.as_deref(), axis))
        {
            bounds = [bounds[0].min(range[0]), bounds[1].max(range[1])];
        } else if matches!(self.shot.mode, CameraMode::Path { .. }) {
            // The original source keys remain support even when the visible interval has none.
            for pair in self.shot.path_keys().windows(2) {
                let low = pair[0].at.max(self.shot.path_offset);
                let high = pair[1]
                    .at
                    .min(MediaTime(self.shot.path_offset.0 + span.duration().0));
                if high < low {
                    continue;
                }
                for key in pair {
                    let value = channel.value(key.pose);
                    bounds = [bounds[0].min(value), bounds[1].max(value)];
                }
                if matches!(channel, CameraChannel::Yaw | CameraChannel::Roll) {
                    let a = channel.value(pair[0].pose);
                    let b = channel.value(pair[1].pose);
                    let end = a + ((b - a + 180.0).rem_euclid(360.0) - 180.0);
                    if end.is_finite() {
                        bounds = [bounds[0].min(end), bounds[1].max(end)];
                    }
                }
            }
        }
        bounds
    }
}
