//! Deterministic replay-view navigation and projection; no live input or clock enters this domain.

use super::*;

const NEAR: f64 = 0.01;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CameraDragAxis {
    #[default]
    View,
    X,
    Y,
    Z,
}

impl CameraDragAxis {
    pub const WORLD: [Self; 3] = [Self::X, Self::Y, Self::Z];

    /// The view-plane tool has no world axis; constrained tools map to one coordinate only.
    pub const fn index(self) -> Option<usize> {
        match self {
            Self::View => None,
            Self::X => Some(0),
            Self::Y => Some(1),
            Self::Z => Some(2),
        }
    }

    /// Compact labels identify a world axis or the original view-plane drag tool.
    pub const fn label(self) -> &'static str {
        match self {
            Self::View => "View",
            Self::X => "X",
            Self::Y => "Y",
            Self::Z => "Z",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CameraSnap {
    #[default]
    Off,
    Tenth,
    Half,
    Block,
}

impl CameraSnap {
    /// Grid resolution is transient authoring state and does not quantize saved camera playback.
    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::Tenth,
            Self::Tenth => Self::Half,
            Self::Half => Self::Block,
            Self::Block => Self::Off,
        }
    }

    /// World-space grid increments are shared by drag math and the displayed selector.
    pub const fn step(self) -> Option<f64> {
        match self {
            Self::Off => None,
            Self::Tenth => Some(0.1),
            Self::Half => Some(0.5),
            Self::Block => Some(1.0),
        }
    }

    /// UI labels describe the same increment used by coordinate quantization.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Tenth => "0.1",
            Self::Half => "0.5",
            Self::Block => "1 block",
        }
    }
}

/// An unsaved inspection view, identified separately from document and replay-seek revisions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraView {
    pub pose: CameraPose,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct CameraProjection {
    position: DVec3,
    right: DVec3,
    up: DVec3,
    forward: DVec3,
    half_height: f64,
    half_width: f64,
}

impl CameraProjection {
    /// Normalized fly motion follows the rolled view horizontally and the world vertically.
    pub(super) fn movement(&self, movement: [f64; 3]) -> DVec3 {
        let movement = DVec3::from_array(movement);
        let movement = movement / movement.length().max(1.0);
        let delta = self.right * movement.x + DVec3::Y * movement.y + self.forward * movement.z;
        delta / delta.length().max(1.0)
    }
    /// Uses the renderer's Bedrock YXZ yaw/pitch/roll convention and a vertical field of view.
    pub fn new(pose: CameraPose, aspect: f64) -> Option<Self> {
        pose.validate().ok()?;
        if !aspect.is_finite() || aspect <= 0.0 {
            return None;
        }
        let (yaw_sin, yaw_cos) = pose.yaw.rem_euclid(360.0).to_radians().sin_cos();
        let (pitch_sin, pitch_cos) = pose.pitch.to_radians().sin_cos();
        let (roll_sin, roll_cos) = pose.roll.rem_euclid(360.0).to_radians().sin_cos();
        let forward = DVec3::new(-yaw_sin * pitch_cos, -pitch_sin, yaw_cos * pitch_cos);
        let right = DVec3::new(-yaw_cos, 0.0, -yaw_sin);
        let up = right.cross(forward);
        let half_height = (pose.fov.to_radians() * 0.5).tan();
        let half_width = half_height * aspect;
        if !half_width.is_finite() || half_width <= 0.0 {
            return None;
        }
        Some(Self {
            position: DVec3::from_array(pose.position),
            right: right * roll_cos + up * roll_sin,
            up: up * roll_cos - right * roll_sin,
            forward,
            half_height,
            half_width,
        })
    }

    /// Returns normalized image coordinates; off-image points remain available for segment clipping.
    pub fn project(&self, point: [f64; 3]) -> Option<[f64; 2]> {
        let delta = DVec3::from_array(point) - self.position;
        let depth = delta.dot(self.forward);
        if !delta.is_finite() || depth < NEAR * (1.0 - 1e-8) {
            return None;
        }
        let projected = [
            0.5 + delta.dot(self.right) / (2.0 * depth * self.half_width),
            0.5 - delta.dot(self.up) / (2.0 * depth * self.half_height),
        ];
        projected
            .iter()
            .all(|value| value.is_finite())
            .then_some(projected)
    }

    /// Clips a real world segment against the near plane and image bounds before drawing a path.
    pub fn segment(&self, a: [f64; 3], b: [f64; 3]) -> Option<[[f64; 2]; 2]> {
        let (mut a, mut b) = (DVec3::from_array(a), DVec3::from_array(b));
        if !a.is_finite() || !b.is_finite() {
            return None;
        }
        let (da, db) = (
            (a - self.position).dot(self.forward),
            (b - self.position).dot(self.forward),
        );
        if da < NEAR && db < NEAR {
            return None;
        }
        if da < NEAR {
            a = a.lerp(b, (NEAR - da) / (db - da));
        } else if db < NEAR {
            b = a.lerp(b, (NEAR - da) / (db - da));
        }
        let [a, b] = [self.project(a.to_array())?, self.project(b.to_array())?];
        let mut low: f64 = 0.0;
        let mut high: f64 = 1.0;
        for axis in 0..2 {
            let delta = b[axis] - a[axis];
            if delta.abs() < 1e-12 {
                if !(0.0..=1.0).contains(&a[axis]) {
                    return None;
                }
            } else {
                let (first, last) = ((-a[axis]) / delta, (1.0 - a[axis]) / delta);
                low = low.max(first.min(last));
                high = high.min(first.max(last));
            }
        }
        (low <= high).then(|| {
            [
                std::array::from_fn(|axis| a[axis] + (b[axis] - a[axis]) * low),
                std::array::from_fn(|axis| a[axis] + (b[axis] - a[axis]) * high),
            ]
        })
    }

    /// Moves a control on the frozen view plane through its original depth, avoiding snap-to-camera.
    pub fn dragged(&self, point: [f64; 3], delta: [f64; 2]) -> Option<[f64; 3]> {
        let point = DVec3::from_array(point);
        let depth = (point - self.position).dot(self.forward);
        if depth < NEAR || !delta.iter().all(|value| value.is_finite()) {
            return None;
        }
        let next = point + self.right * (delta[0] * 2.0 * depth * self.half_width)
            - self.up * (delta[1] * 2.0 * depth * self.half_height);
        next.is_finite().then(|| next.to_array())
    }

    /// Closest ray/axis dragging preserves the other world coordinates; near-parallel axes stay still.
    pub fn dragged_constrained(
        &self,
        point: [f64; 3],
        delta: [f64; 2],
        axis: CameraDragAxis,
        snap: CameraSnap,
    ) -> Option<[f64; 3]> {
        if !delta.iter().all(|value| value.is_finite()) {
            return None;
        }
        let projected = self.project(point)?;
        if delta == [0.0; 2] {
            return Some(point);
        }
        let mut next = if let Some(index) = axis.index() {
            let cursor = [projected[0] + delta[0], projected[1] + delta[1]];
            let ray = (self.forward
                + self.right * ((cursor[0] * 2.0 - 1.0) * self.half_width)
                + self.up * ((1.0 - cursor[1] * 2.0) * self.half_height))
                .normalize();
            let direction = match index {
                0 => DVec3::X,
                1 => DVec3::Y,
                _ => DVec3::Z,
            };
            let cosine = direction.dot(ray);
            let denominator = 1.0 - cosine * cosine;
            if denominator < 1e-8 {
                return None;
            }
            let offset = DVec3::from_array(point) - self.position;
            let distance = (cosine * ray.dot(offset) - direction.dot(offset)) / denominator;
            let mut next = point;
            next[index] += distance;
            next
        } else {
            self.dragged(point, delta)?
        };
        if let Some(step) = snap.step() {
            if let Some(index) = axis.index() {
                next[index] = (next[index] / step).round() * step;
            } else {
                next = next.map(|value| (value / step).round() * step);
            }
        }
        next.iter().all(|value| value.is_finite()).then_some(next)
    }
}

impl CameraPose {
    /// Frames a recorded point with explicit half-height while retaining viewing direction, roll and lens.
    pub fn framed(self, center: [f64; 3], half_height: f64) -> Option<Self> {
        self.validate().ok()?;
        if !center.into_iter().chain([half_height]).all(f64::is_finite) || half_height <= 0.0 {
            return None;
        }
        let center = DVec3::from_array(center);
        let mut direction = DVec3::from_array(self.position) - center;
        if direction.length_squared() < 1e-8 {
            direction =
                DVec3::from_array(self.position) - DVec3::from_array(self.orbit_anchor(1.0)?);
        }
        let distance = (half_height / (self.fov.to_radians() * 0.5).tan()).clamp(2.0, 24.0);
        Some(
            Self {
                position: (center + direction.normalize() * distance).to_array(),
                ..self
            }
            .looking_at(center.to_array()),
        )
    }

    /// Chooses a view-relative orbit pivot when no recorded subject is selected.
    pub fn orbit_anchor(self, distance: f64) -> Option<[f64; 3]> {
        if !distance.is_finite() || distance <= 0.0 {
            return None;
        }
        let basis = CameraProjection::new(self, 1.0)?;
        Some((basis.position + basis.forward * distance).to_array())
    }

    /// Rotates around a frozen pivot, preserving radius, framing offset and the original roll.
    pub fn orbited(self, pivot: [f64; 3], look: [f64; 2]) -> Option<Self> {
        self.validate().ok()?;
        if !pivot.into_iter().chain(look).all(f64::is_finite) {
            return None;
        }
        if look == [0.0; 2] {
            return Some(self);
        }
        let delta = DVec3::from_array(pivot) - DVec3::from_array(self.position);
        let radius = delta.length();
        if radius < NEAR {
            return self.flown([0.0; 3], look, 0.0, 0.0);
        }
        let centered = self.looking_at(pivot);
        let pitch = (centered.pitch + look[1]).clamp(-89.5, 89.5);
        let turn = [look[0], pitch - centered.pitch];
        let direction = CameraProjection::new(
            Self {
                yaw: centered.yaw + turn[0],
                pitch,
                ..centered
            },
            1.0,
        )?
        .forward;
        let next = Self {
            position: (DVec3::from_array(pivot) - direction * radius).to_array(),
            yaw: (self.yaw + turn[0]).rem_euclid(360.0),
            pitch: (self.pitch + turn[1]).clamp(-89.9, 89.9),
            ..self
        };
        next.validate().ok()?;
        Some(next)
    }

    /// Integrates only caller-supplied navigation input, with normalized diagonals and bounded frame time.
    pub fn flown(
        self,
        movement: [f64; 3],
        look: [f64; 2],
        elapsed: f64,
        speed: f64,
    ) -> Option<Self> {
        if !movement
            .into_iter()
            .chain(look)
            .chain([elapsed, speed])
            .all(f64::is_finite)
            || elapsed < 0.0
            || speed < 0.0
        {
            return None;
        }
        let mut next = self;
        next.yaw = (next.yaw + look[0]).rem_euclid(360.0);
        if look[1] != 0.0 {
            next.pitch = (next.pitch + look[1]).clamp(-89.9, 89.9);
        }
        let basis = CameraProjection::new(next, 1.0)?;
        let delta = basis.movement(movement);
        next.position =
            (DVec3::from_array(self.position) + delta * elapsed.min(0.1) * speed).to_array();
        next.validate().ok()?;
        Some(next)
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;

    #[test]
    fn vertical_views_keep_their_pitch_without_vertical_look_input() {
        for pitch in [-90.0, 90.0] {
            let pose = CameraPose {
                pitch,
                ..Default::default()
            };
            for movement in [[0.0; 3], [1.0, 0.0, 1.0]] {
                let next = pose.flown(movement, [0.0; 2], 0.05, 8.0).unwrap();
                assert_eq!(next.pitch, pitch);
            }
        }
    }
}
