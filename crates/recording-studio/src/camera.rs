//! Editable replay camera rigs, evaluated from recorded actor poses at the requested time.

use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::{Id, MediaTime, Span, StudioError, effect::Easing, error::limit};

pub const MAX_CAMERA_KEYS: usize = 8192;

/// Network-facing angles are degrees: yaw zero faces +Z and positive pitch looks down.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraPose {
    pub position: [f64; 3],
    pub yaw: f64,
    pub pitch: f64,
    pub roll: f64,
    pub fov: f64,
}

impl Default for CameraPose {
    fn default() -> Self {
        Self {
            position: [0.0, 64.0, 0.0],
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            fov: 70.0,
        }
    }
}

impl CameraPose {
    /// Rejects values which cannot form a finite view or projection.
    pub fn validate(self) -> Result<(), StudioError> {
        if !self
            .position
            .into_iter()
            .chain([self.yaw, self.pitch, self.roll, self.fov])
            .all(f64::is_finite)
            || !(1.0..179.0).contains(&self.fov)
            || self.pitch.abs() > 90.0
        {
            return Err(StudioError::Invalid(
                "camera pose must be finite with a valid pitch and field of view",
            ));
        }
        Ok(())
    }

    /// Points a rig at its recorded target; a coincident target preserves its last orientation.
    pub fn looking_at(mut self, target: [f64; 3]) -> Self {
        let delta = DVec3::from_array(target) - DVec3::from_array(self.position);
        if delta.length_squared() > 1e-12 {
            self.yaw = (-delta.x).atan2(delta.z).to_degrees();
            self.pitch = (-delta.y).atan2(delta.x.hypot(delta.z)).to_degrees();
        }
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraKey {
    /// Offset within the shot, independent of where the shot sits on the timeline.
    pub at: MediaTime,
    pub pose: CameraPose,
    pub easing: Easing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CameraMode {
    Player {
        actor: u64,
    },
    Free {
        pose: CameraPose,
    },
    Path {
        keys: Vec<CameraKey>,
        smooth_position: bool,
    },
    Follow {
        actor: u64,
        offset: [f64; 3],
        look_ahead: f64,
        fov: f64,
    },
    Orbit {
        actor: u64,
        radius: f64,
        height: f64,
        start_degrees: f64,
        turns: f64,
        fov: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraShot {
    pub id: Id,
    pub name: String,
    pub span: Span,
    pub mode: CameraMode,
    pub hide_hud: bool,
    pub hide_hand: bool,
}

/// A target sample comes from replay state; sampling never queries the live server.
#[derive(Clone, Copy, Debug)]
pub struct ActorPose {
    pub position: [f64; 3],
    pub velocity: [f64; 3],
    pub yaw: f64,
    pub pitch: f64,
    pub eye_height: f64,
}

impl CameraShot {
    /// Checks rigs and every path key once at project admission or edit commit.
    pub fn validate(&self) -> Result<(), StudioError> {
        self.span.validate()?;
        limit("camera name bytes", self.name.len(), 256)?;
        let valid_fov = |fov: f64| fov.is_finite() && (1.0..179.0).contains(&fov);
        match &self.mode {
            CameraMode::Player { .. } => (),
            CameraMode::Free { pose } => pose.validate()?,
            CameraMode::Path { keys, .. } => {
                limit("camera keys", keys.len(), MAX_CAMERA_KEYS)?;
                if keys.is_empty()
                    || keys.windows(2).any(|pair| pair[0].at >= pair[1].at)
                    || keys.last().is_some_and(|key| key.at > self.span.duration())
                {
                    return Err(StudioError::Invalid(
                        "camera keys must be ordered within their shot",
                    ));
                }
                for key in keys {
                    key.pose.validate()?;
                }
            }
            CameraMode::Follow {
                offset,
                look_ahead,
                fov,
                ..
            } => {
                if !offset.iter().all(|value| value.is_finite())
                    || !look_ahead.is_finite()
                    || !(0.0..=10.0).contains(look_ahead)
                    || !valid_fov(*fov)
                {
                    return Err(StudioError::Invalid("follow camera parameters are invalid"));
                }
            }
            CameraMode::Orbit {
                radius,
                height,
                start_degrees,
                turns,
                fov,
                ..
            } => {
                if ![radius, height, start_degrees, turns]
                    .iter()
                    .all(|value| value.is_finite())
                    || !(0.1..=1024.0).contains(radius)
                    || height.abs() > 1024.0
                    || turns.abs() > 1024.0
                    || !valid_fov(*fov)
                {
                    return Err(StudioError::Invalid("orbit camera parameters are invalid"));
                }
            }
        }
        Ok(())
    }

    /// Evaluates a shot without accumulated integration, making scrubbing and export agree.
    pub fn sample(
        &self,
        time: MediaTime,
        actor: impl Fn(u64) -> Option<ActorPose>,
    ) -> Option<CameraPose> {
        if !self.span.contains(time) {
            return None;
        }
        let elapsed = time.saturating_sub(self.span.start);
        match &self.mode {
            CameraMode::Free { pose } => Some(*pose),
            CameraMode::Path {
                keys,
                smooth_position,
            } => sample_path(keys, elapsed, *smooth_position),
            CameraMode::Player { actor: target } => {
                let actor = actor(*target)?;
                let mut pose = CameraPose {
                    position: actor.position,
                    yaw: actor.yaw,
                    pitch: actor.pitch,
                    ..CameraPose::default()
                };
                pose.position[1] += actor.eye_height;
                Some(pose)
            }
            CameraMode::Follow {
                actor: target,
                offset,
                look_ahead,
                fov,
            } => {
                let target = actor(*target)?;
                let yaw = target.yaw.to_radians();
                let (sin, cos) = yaw.sin_cos();
                let local = DVec3::new(
                    offset[0] * cos - offset[2] * sin,
                    offset[1],
                    offset[0] * sin + offset[2] * cos,
                );
                let centre = DVec3::from_array(target.position) + DVec3::Y * target.eye_height;
                let look = centre + DVec3::from_array(target.velocity) * *look_ahead;
                Some(
                    CameraPose {
                        position: (centre + local).to_array(),
                        fov: *fov,
                        ..CameraPose::default()
                    }
                    .looking_at(look.to_array()),
                )
            }
            CameraMode::Orbit {
                actor: target,
                radius,
                height,
                start_degrees,
                turns,
                fov,
            } => {
                let actor = actor(*target)?;
                let centre = DVec3::from_array(actor.position) + DVec3::Y * actor.eye_height;
                let progress = elapsed.0 as f64 / self.span.duration().0 as f64;
                let angle = (start_degrees + progress * turns * 360.0).to_radians();
                let position =
                    centre + DVec3::new(angle.sin() * radius, *height, angle.cos() * radius);
                Some(
                    CameraPose {
                        position: position.to_array(),
                        fov: *fov,
                        ..CameraPose::default()
                    }
                    .looking_at(centre.to_array()),
                )
            }
        }
    }
}

/// Interpolates angle wrap on the shorter arc rather than turning a camera almost 360 degrees.
fn angle(a: f64, b: f64, amount: f64) -> f64 {
    a + ((b - a + 180.0).rem_euclid(360.0) - 180.0) * amount
}

/// Samples bounded path keys using eased orientation and optional clamped Hermite positions.
fn sample_path(keys: &[CameraKey], time: MediaTime, smooth: bool) -> Option<CameraPose> {
    let next = keys.partition_point(|key| key.at <= time);
    if next == 0 {
        return keys.first().map(|key| key.pose);
    }
    if next == keys.len() {
        return keys.last().map(|key| key.pose);
    }
    let (a, b) = (&keys[next - 1], &keys[next]);
    let t = b
        .easing
        .sample((time.0 - a.at.0) as f64 / (b.at.0 - a.at.0) as f64);
    let mut position =
        DVec3::from_array(a.pose.position).lerp(DVec3::from_array(b.pose.position), t);
    if smooth {
        let p0 = DVec3::from_array(keys[next.saturating_sub(2)].pose.position);
        let p1 = DVec3::from_array(a.pose.position);
        let p2 = DVec3::from_array(b.pose.position);
        let p3 = DVec3::from_array(keys[(next + 1).min(keys.len() - 1)].pose.position);
        position = 0.5
            * ((2.0 * p1)
                + (-p0 + p2) * t
                + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
                + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t * t * t);
        // A rail may curve, but never leave the segment's bounding box at a sharp key turn.
        position = position.clamp(p1.min(p2), p1.max(p2));
    }
    Some(CameraPose {
        position: position.to_array(),
        yaw: angle(a.pose.yaw, b.pose.yaw, t),
        pitch: a.pose.pitch + (b.pose.pitch - a.pose.pitch) * t,
        roll: angle(a.pose.roll, b.pose.roll, t),
        fov: a.pose.fov + (b.pose.fov - a.pose.fov) * t,
    })
}
