//! Editable replay camera rigs, evaluated from recorded actor poses at the requested time.

use glam::DVec3;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{Id, MediaTime, Span, StudioError, effect::Easing, error::limit};

pub const MAX_CAMERA_KEYS: usize = 8192;
pub const MAX_CAMERA_TARGETS: usize = 4096;
pub const MAX_TARGET_NAME_BYTES: usize = 256;
pub const CAMERA_FOV_RANGE: std::ops::Range<f64> = 1.0..179.0;
const DEFAULT_FOV: f64 = 70.0;

mod animation;
mod continuity;
pub mod coverage;
mod distance;
mod edit;
mod fov;
mod keys;
pub mod navigation;
mod path;
mod presets;
mod rotation;
mod saved;
mod subjects;
mod viewport;
pub use animation::{CameraAnimation, CameraChannel, CameraChannelCurve};
pub use continuity::PathContinuity;
pub use distance::{CompiledCameraPath, PathAccuracy, PathTiming};
pub use edit::{CameraParameter, CameraRigKind};
pub use path::{CameraHandles, PathControl};
pub use presets::CameraPreset;
pub use rotation::{CameraRotationAxis, CameraRotationDrag};
pub use saved::{CameraViewSource, MAX_SAVED_VIEW_NAME_BYTES, SavedCameraView};
pub use subjects::{
    CameraSubjectFilter, CameraSubjectSearch, CameraTargetKind, LEGACY_SUBJECT_NOTICE,
};
pub use viewport::{CameraDragAxis, CameraProjection, CameraSnap, CameraView};

#[cfg(test)]
mod viewport_tests;

/// Numeric drafts allow ordinary decimal and scientific notation while retaining a fixed input bound.
pub const MAX_CAMERA_VALUE_BYTES: usize = 64;

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
            fov: DEFAULT_FOV,
        }
    }
}

impl CameraPose {
    /// Samples finite admitted poses with the shortest yaw and roll arcs and exact endpoints.
    pub fn interpolated(self, other: Self, amount: f64) -> Self {
        let t = amount.clamp(0.0, 1.0);
        if t == 0.0 {
            return self;
        }
        if t == 1.0 {
            return other;
        }
        Self {
            position: DVec3::from_array(self.position)
                .lerp(DVec3::from_array(other.position), t)
                .to_array(),
            yaw: angle(self.yaw, other.yaw, t),
            pitch: self.pitch + (other.pitch - self.pitch) * t,
            roll: angle(self.roll, other.roll, t),
            fov: self.fov + (other.fov - self.fov) * t,
        }
    }

    /// Rejects values which cannot form a finite view or projection.
    pub fn validate(self) -> Result<(), StudioError> {
        if !self
            .position
            .into_iter()
            .chain([self.yaw, self.pitch, self.roll, self.fov])
            .all(f64::is_finite)
            || !CAMERA_FOV_RANGE.contains(&self.fov)
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
    /// Original path time; the shot's path offset preserves this source clock across trims.
    pub at: MediaTime,
    pub pose: CameraPose,
    pub easing: Easing,
    /// Relative world-space Bezier controls; absent controls preserve legacy rail evaluation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handles: Option<CameraHandles>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CameraMode {
    Player {
        actor: u64,
        #[serde(default = "default_fov")]
        fov: f64,
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
        #[serde(default)]
        roll: f64,
    },
    Track {
        actor: u64,
        pose: CameraPose,
    },
    Orbit {
        actor: u64,
        radius: f64,
        height: f64,
        start_degrees: f64,
        turns: f64,
        fov: f64,
        #[serde(default)]
        roll: f64,
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
    /// Independent graph channels override the sampled rig without rewriting its positional rail.
    #[serde(default, skip_serializing_if = "CameraAnimation::is_empty")]
    pub animation: CameraAnimation,
    #[serde(default)]
    pub target_lifetime: Option<TargetLifetime>,
    /// Independent lens animation overrides the rig's original FOV only when explicitly authored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fov_curve: Option<crate::effect::Curve>,
    /// A trim retains the original easing phase without resampling or changing interior lens motion.
    #[serde(default)]
    pub fov_offset: MediaTime,
    /// Distance timing keeps spatial speed uniform between authored keys; lens/orientation keep their easing.
    #[serde(default, skip_serializing_if = "PathTiming::is_keyed")]
    pub path_timing: PathTiming,
    /// Source-time phase retains original support keys and exact easing when a path is trimmed.
    #[serde(default)]
    pub path_offset: MediaTime,
    /// Whole-rail travel easing leaves authored orientation and lens clocks independent.
    #[serde(
        default = "distance::linear_easing",
        skip_serializing_if = "distance::is_linear"
    )]
    pub path_global_easing: Easing,
    /// Aligned geometric tangents are opt-in so existing authored rails retain their exact shape.
    #[serde(default, skip_serializing_if = "PathContinuity::is_authored")]
    pub path_continuity: PathContinuity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetLifetime {
    pub unique_id: i64,
    pub spawn_revision: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CameraTargetRef {
    pub runtime_id: u64,
    pub lifetime: Option<TargetLifetime>,
}

/// A target sample comes from replay state; sampling never queries the live server.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActorPose {
    pub position: [f64; 3],
    pub velocity: [f64; 3],
    pub yaw: f64,
    pub pitch: f64,
    pub roll: f64,
    /// A missing eye offset allows root/centre tracking but never invents a first-person view.
    pub eye_offset: Option<[f64; 3]>,
}

impl ActorPose {
    /// Existing target anchors are native eyes when recorded, otherwise their recorded centre.
    pub fn centre(self) -> [f64; 3] {
        std::array::from_fn(|axis| self.position[axis] + self.eye_offset.unwrap_or([0.0; 3])[axis])
    }
    /// Interpolates recorded target motion without deriving a view from a missing native eye offset.
    pub fn interpolated(self, other: Self, amount: f64) -> Self {
        let t = amount.clamp(0.0, 1.0);
        if t == 0.0 {
            return self;
        }
        if t == 1.0 {
            return other;
        }
        Self {
            position: DVec3::from_array(self.position)
                .lerp(DVec3::from_array(other.position), t)
                .to_array(),
            velocity: DVec3::from_array(self.velocity)
                .lerp(DVec3::from_array(other.velocity), t)
                .to_array(),
            yaw: angle(self.yaw, other.yaw, t),
            pitch: self.pitch + (other.pitch - self.pitch) * t,
            roll: angle(self.roll, other.roll, t),
            eye_offset: match (self.eye_offset, other.eye_offset) {
                (Some(a), Some(b)) => Some(
                    DVec3::from_array(a)
                        .lerp(DVec3::from_array(b), t)
                        .to_array(),
                ),
                _ => None,
            },
        }
    }

    /// Capture and playback reject invalid arithmetic before it can reach a camera matrix.
    pub fn validate(self) -> Result<(), StudioError> {
        if !self
            .position
            .into_iter()
            .chain(self.velocity)
            .chain([self.yaw, self.pitch, self.roll])
            .chain(self.eye_offset.into_iter().flatten())
            .all(f64::is_finite)
            || self.pitch.abs() > 90.0
        {
            return Err(StudioError::Invalid(
                "camera target pose is not finite or has an invalid pitch",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CameraTarget {
    pub runtime_id: u64,
    pub unique_id: i64,
    pub spawn_revision: u64,
    pub name: Arc<str>,
    pub kind: CameraTargetKind,
    pub entity_type: Option<Arc<str>>,
    pub pose: ActorPose,
    pub teleported: bool,
}

impl CameraTarget {
    /// Labels are local media metadata with an explicit bound, independent of a server's text limits.
    pub fn validate(&self) -> Result<(), StudioError> {
        limit(
            "camera target name bytes",
            self.name.len(),
            MAX_TARGET_NAME_BYTES,
        )?;
        if let Some(entity_type) = &self.entity_type {
            limit(
                "camera target entity type bytes",
                entity_type.len(),
                MAX_TARGET_NAME_BYTES,
            )?;
            if entity_type.is_empty() || entity_type.chars().any(char::is_control) {
                return Err(StudioError::Invalid(
                    "camera target entity type is empty or contains control characters",
                ));
            }
        }
        self.pose.validate()
    }

    /// Runtime ID reuse never turns a camera's position difference into a false velocity sample.
    pub fn same_lifetime(&self, other: &Self) -> bool {
        (self.runtime_id, self.unique_id, self.spawn_revision)
            == (other.runtime_id, other.unique_id, other.spawn_revision)
    }

    /// Editor-authored shots bind a spawn lifetime so runtime-ID reuse cannot select a different subject.
    pub fn reference(&self) -> CameraTargetRef {
        CameraTargetRef {
            runtime_id: self.runtime_id,
            lifetime: Some(TargetLifetime {
                unique_id: self.unique_id,
                spawn_revision: self.spawn_revision,
            }),
        }
    }

    /// Legacy runtime-only references remain readable; newly authored shots always carry their lifetime.
    pub fn matches(&self, reference: CameraTargetRef) -> bool {
        self.runtime_id == reference.runtime_id
            && reference
                .lifetime
                .is_none_or(|lifetime| self.reference().lifetime == Some(lifetime))
    }
}

impl CameraShot {
    /// Every subject-bound rig uses the same lifetime reference for admission and frame sampling.
    pub fn target_reference(&self) -> Option<CameraTargetRef> {
        let actor = match &self.mode {
            CameraMode::Player { actor, .. }
            | CameraMode::Follow { actor, .. }
            | CameraMode::Track { actor, .. }
            | CameraMode::Orbit { actor, .. } => *actor,
            CameraMode::Free { .. } | CameraMode::Path { .. } => return None,
        };
        Some(CameraTargetRef {
            runtime_id: actor,
            lifetime: self.target_lifetime,
        })
    }

    /// First-person subject visibility is independent of whether recorded HUD layers are shown.
    pub fn spectated_actor(&self) -> Option<CameraTargetRef> {
        if let CameraMode::Player { actor, .. } = &self.mode {
            Some(CameraTargetRef {
                runtime_id: *actor,
                lifetime: self.target_lifetime,
            })
        } else {
            None
        }
    }

    /// Checks rigs and every path key once at project admission or edit commit.
    pub fn validate(&self) -> Result<(), StudioError> {
        self.span.validate()?;
        self.animation.validate(self.span.duration())?;
        limit("camera name bytes", self.name.len(), 256)?;
        let valid_fov = |fov: f64| fov.is_finite() && CAMERA_FOV_RANGE.contains(&fov);
        if let Some(curve) = &self.fov_curve {
            curve.validate(MediaTime(crate::time::MAX_MEDIA_MICROS))?;
            self.fov_offset.checked_add(self.span.duration())?;
            if curve.keys.iter().any(|key| !valid_fov(key.value)) {
                return Err(StudioError::Invalid(
                    "camera FOV keys must be between 1 and 179 degrees",
                ));
            }
        }
        if self.fov_curve.is_none() && self.fov_offset != MediaTime::ZERO {
            return Err(StudioError::Invalid(
                "a camera FOV phase requires a lens curve",
            ));
        }
        if self.path_timing != PathTiming::Keyed && !matches!(self.mode, CameraMode::Path { .. }) {
            return Err(StudioError::Invalid(
                "distance travel timing requires a camera path",
            ));
        }
        if matches!(self.mode, CameraMode::Path { .. }) {
            self.path_offset.checked_add(self.span.duration())?;
        } else if self.path_offset != MediaTime::ZERO
            || self.path_continuity != PathContinuity::Authored
            || self.path_global_easing != Easing::Linear
        {
            return Err(StudioError::Invalid(
                "a path source phase requires a camera rail",
            ));
        }
        match &self.mode {
            CameraMode::Player { fov, actor: _ } => {
                if !valid_fov(*fov) {
                    return Err(StudioError::Invalid(
                        "player camera field of view is invalid",
                    ));
                }
            }
            CameraMode::Free { pose } | CameraMode::Track { pose, .. } => pose.validate()?,
            CameraMode::Path {
                keys,
                smooth_position,
            } => {
                limit("camera keys", keys.len(), MAX_CAMERA_KEYS)?;
                if keys.is_empty()
                    || keys.windows(2).any(|pair| pair[0].at >= pair[1].at)
                    || keys
                        .last()
                        .is_some_and(|key| key.at.0 > crate::time::MAX_MEDIA_MICROS)
                {
                    return Err(StudioError::Invalid(
                        "camera keys must be ordered within their bounded source clock",
                    ));
                }
                for key in keys {
                    key.pose.validate()?;
                    if let Some(handles) = &key.handles {
                        handles.validate(key.pose.position)?;
                    }
                }
                if self.path_timing != PathTiming::Keyed
                    || self.path_continuity != PathContinuity::Authored
                {
                    distance::validate(keys, *smooth_position, self.path_continuity)?;
                }
            }
            CameraMode::Follow {
                offset,
                look_ahead,
                fov,
                roll,
                ..
            } => {
                if !offset.iter().all(|value| value.is_finite())
                    || !look_ahead.is_finite()
                    || !(0.0..=10.0).contains(look_ahead)
                    || !valid_fov(*fov)
                    || !roll.is_finite()
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
                roll,
                ..
            } => {
                if ![radius, height, start_degrees, turns, roll]
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
        if self.target_lifetime.is_some()
            && matches!(self.mode, CameraMode::Free { .. } | CameraMode::Path { .. })
        {
            return Err(StudioError::Invalid(
                "a camera without an actor cannot bind a target lifetime",
            ));
        }
        Ok(())
    }

    /// Evaluates a shot without accumulated integration, making scrubbing and export agree.
    pub fn sample(
        &self,
        time: MediaTime,
        actor: impl Fn(CameraTargetRef) -> Option<ActorPose>,
    ) -> Option<CameraPose> {
        self.sample_compiled(time, actor, None)
    }

    /// Frame evaluation reuses the immutable rail's arc cache; standalone callers retain identical output.
    pub(crate) fn sample_compiled(
        &self,
        time: MediaTime,
        actor: impl Fn(CameraTargetRef) -> Option<ActorPose>,
        path: Option<&CompiledCameraPath>,
    ) -> Option<CameraPose> {
        if !self.span.contains(time) {
            return None;
        }
        let elapsed = time.saturating_sub(self.span.start);
        let mut pose = match &self.mode {
            CameraMode::Free { pose } => Some(*pose),
            CameraMode::Track {
                actor: target,
                pose,
            } => {
                let target = actor(CameraTargetRef {
                    runtime_id: *target,
                    lifetime: self.target_lifetime,
                })?;
                Some(pose.looking_at(target.centre()))
            }
            CameraMode::Path {
                keys,
                smooth_position,
            } => {
                let elapsed = self.path_offset.checked_add(elapsed).ok()?;
                match self.path_timing {
                    PathTiming::GlobalDistance => path
                        .and_then(|path| path.sample(time))
                        .or_else(|| {
                            CompiledCameraPath::new(self).and_then(|path| path.sample(time))
                        })
                        .or_else(|| keys.first().map(|key| key.pose)),
                    PathTiming::Distance => {
                        path.map(|path| path.sample(time)).unwrap_or_else(|| {
                            distance::sample(keys, elapsed, *smooth_position, self.path_continuity)
                        })
                    }
                    PathTiming::Keyed => {
                        sample_path_policy(keys, elapsed, *smooth_position, self.path_continuity)
                    }
                }
            }
            CameraMode::Player { actor: target, fov } => {
                let actor = actor(CameraTargetRef {
                    runtime_id: *target,
                    lifetime: self.target_lifetime,
                })?;
                let mut pose = CameraPose {
                    position: actor.position,
                    yaw: actor.yaw,
                    pitch: actor.pitch,
                    roll: actor.roll,
                    fov: *fov,
                };
                pose.position = (DVec3::from_array(pose.position)
                    + DVec3::from_array(actor.eye_offset?))
                .to_array();
                Some(pose)
            }
            CameraMode::Follow {
                actor: target,
                offset,
                look_ahead,
                fov,
                roll,
            } => {
                let target = actor(CameraTargetRef {
                    runtime_id: *target,
                    lifetime: self.target_lifetime,
                })?;
                let yaw = target.yaw.to_radians();
                let (sin, cos) = yaw.sin_cos();
                let local = DVec3::new(
                    offset[0] * cos - offset[2] * sin,
                    offset[1],
                    offset[0] * sin + offset[2] * cos,
                );
                let centre = DVec3::from_array(target.position)
                    + DVec3::from_array(target.eye_offset.unwrap_or([0.0; 3]));
                let look = centre + DVec3::from_array(target.velocity) * *look_ahead;
                Some(
                    CameraPose {
                        position: (centre + local).to_array(),
                        yaw: target.yaw,
                        pitch: target.pitch,
                        fov: *fov,
                        roll: *roll,
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
                roll,
            } => {
                let actor = actor(CameraTargetRef {
                    runtime_id: *target,
                    lifetime: self.target_lifetime,
                })?;
                let centre = DVec3::from_array(actor.position)
                    + DVec3::from_array(actor.eye_offset.unwrap_or([0.0; 3]));
                let progress = elapsed.0 as f64 / self.span.duration().0 as f64;
                let angle = (start_degrees + progress * turns * 360.0).to_radians();
                let position =
                    centre + DVec3::new(angle.sin() * radius, *height, angle.cos() * radius);
                Some(
                    CameraPose {
                        position: position.to_array(),
                        fov: *fov,
                        roll: *roll,
                        ..CameraPose::default()
                    }
                    .looking_at(centre.to_array()),
                )
            }
        }?;
        self.animation.apply(&mut pose, elapsed);
        if let Some(fov) = self.fov_override(time) {
            pose.fov = fov;
        }
        Some(pose)
    }
}

/// Older player-camera projects inherit the same default as an ordinary camera pose.
fn default_fov() -> f64 {
    DEFAULT_FOV
}

/// Interpolates angle wrap on the shorter arc rather than turning a camera almost 360 degrees.
fn angle(a: f64, b: f64, amount: f64) -> f64 {
    a + ((b.rem_euclid(360.0) - a.rem_euclid(360.0) + 180.0).rem_euclid(360.0) - 180.0) * amount
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
    let mut pose = a.pose.interpolated(b.pose, t);
    if a.handles
        .as_ref()
        .and_then(|handles| handles.outgoing)
        .is_some()
        || b.handles
            .as_ref()
            .and_then(|handles| handles.incoming)
            .is_some()
    {
        pose.position = path::bezier_position(keys, next - 1, t, smooth, PathContinuity::Authored);
        return Some(pose);
    }
    if smooth {
        pose.position = std::array::from_fn(|axis| {
            let p1 = a.pose.position[axis];
            let p2 = b.pose.position[axis];
            let duration = b.at.saturating_sub(a.at).seconds();
            let slope = (p2 - p1) / duration;
            let first = next.checked_sub(2).map_or(slope, |index| {
                let previous = &keys[index];
                let before = a.at.saturating_sub(previous.at).seconds();
                tangent(
                    (p1 - previous.pose.position[axis]) / before,
                    slope,
                    before,
                    duration,
                )
            });
            let last = keys.get(next + 1).map_or(slope, |following| {
                let after = following.at.saturating_sub(b.at).seconds();
                tangent(
                    slope,
                    (following.pose.position[axis] - p2) / after,
                    duration,
                    after,
                )
            });
            let t2 = t * t;
            let t3 = t2 * t;
            ((2.0 * t3 - 3.0 * t2 + 1.0) * p1
                + (t3 - 2.0 * t2 + t) * first * duration
                + (-2.0 * t3 + 3.0 * t2) * p2
                + (t3 - t2) * last * duration)
                .clamp(p1.min(p2), p1.max(p2))
        });
    }
    Some(pose)
}

/// Aligned tangent mode is explicit; the original sampler stays bit-for-bit unchanged by default.
fn sample_path_policy(
    keys: &[CameraKey],
    time: MediaTime,
    smooth: bool,
    continuity: PathContinuity,
) -> Option<CameraPose> {
    if continuity == PathContinuity::Authored {
        return sample_path(keys, time, smooth);
    }
    let next = keys.partition_point(|key| key.at <= time);
    if next == 0 {
        return keys.first().map(|key| key.pose);
    }
    if next == keys.len() {
        return keys.last().map(|key| key.pose);
    }
    let (a, b) = (&keys[next - 1], &keys[next]);
    let amount = b
        .easing
        .sample((time.0 - a.at.0) as f64 / (b.at.0 - a.at.0) as f64);
    let mut pose = a.pose.interpolated(b.pose, amount);
    pose.position = path::bezier_position(keys, next - 1, amount, smooth, continuity);
    Some(pose)
}

/// Time-weighted monotone tangents keep unevenly spaced rail keys smooth without overshoot at turns.
fn tangent(before: f64, after: f64, before_time: f64, after_time: f64) -> f64 {
    if before == 0.0 || after == 0.0 || before.signum() != after.signum() {
        return 0.0;
    }
    let first_weight = 2.0 * after_time + before_time;
    let last_weight = after_time + 2.0 * before_time;
    (first_weight + last_weight) / (first_weight / before + last_weight / after)
}
