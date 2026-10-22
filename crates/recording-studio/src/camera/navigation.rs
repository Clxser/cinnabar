//! Transient inspection motion, independent of saved rail timing and replay clocks.

use super::{CAMERA_FOV_RANGE, CameraPose, CameraProjection, DVec3};

pub const DEFAULT_SPEED: f64 = 8.0;
pub const FAST_MULTIPLIER: f64 = 4.0;
pub const LOOK_DEGREES_PER_PIXEL: f64 = 0.18;
pub const ORBIT_DEGREES_PER_PIXEL: f64 = 0.22;
pub const DEFAULT_ORBIT_DISTANCE: f64 = 8.0;
pub const WHEEL_PIXELS_PER_STEP: f64 = 40.0;
const HALF_LIFE: f64 = 0.08;
const REST_SPEED: f64 = 1e-5;
const MIN_SPEED: f64 = 0.25;
const MAX_SPEED: f64 = 256.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavigationSettings {
    speed: f64,
    pub smooth: bool,
}

impl Default for NavigationSettings {
    fn default() -> Self {
        Self {
            speed: DEFAULT_SPEED,
            smooth: false,
        }
    }
}

impl NavigationSettings {
    /// Base speed is in blocks per second; fast movement retains the same multiplier at every setting.
    pub const fn speed(self) -> f64 {
        self.speed
    }

    /// Smooth mode is an inspection preference, independent of saved camera easing.
    pub const fn with_smooth(self, smooth: bool) -> Self {
        Self { smooth, ..self }
    }

    /// Restores the original fly speed without changing the motion preference.
    pub const fn reset_speed(self) -> Self {
        Self {
            speed: DEFAULT_SPEED,
            ..self
        }
    }

    /// Four wheel steps double speed; finite editor preferences stay within a useful inspection range.
    pub fn adjusted(self, steps: f64) -> Option<Self> {
        if !steps.is_finite() {
            return None;
        }
        let speed = (self.speed.log2() + steps * 0.25)
            .clamp(MIN_SPEED.log2(), MAX_SPEED.log2())
            .exp2();
        Some(Self { speed, ..self })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NavigationMotion {
    velocity: DVec3,
}

impl NavigationMotion {
    /// Clears momentum without changing the inspected pose or its saved metadata.
    pub fn reset(&mut self) {
        self.velocity = DVec3::ZERO;
    }

    /// Integrates a constant world velocity target analytically over one bounded input interval.
    /// Immediate mode is exactly the existing fly operation. Near rest, remaining travel below
    /// REST_SPEED / lambda is discarded so idle inspection stops publishing subpixel changes.
    pub fn step(
        &mut self,
        pose: CameraPose,
        movement: [f64; 3],
        look: [f64; 2],
        elapsed: f64,
        settings: NavigationSettings,
        fast: bool,
    ) -> Option<CameraPose> {
        let speed = settings.speed * if fast { FAST_MULTIPLIER } else { 1.0 };
        let immediate = pose.flown(movement, look, elapsed, speed)?;
        if !settings.smooth {
            self.reset();
            return Some(immediate);
        }
        let mut next = pose.flown([0.0; 3], look, 0.0, 0.0)?;
        let target = CameraProjection::new(next, 1.0)?.movement(movement) * speed;
        let dt = elapsed.min(0.1);
        let lambda = std::f64::consts::LN_2 / HALF_LIFE;
        let response = -(-lambda * dt).exp_m1();
        let delta = target * dt + (self.velocity - target) * (response / lambda);
        next.position = (DVec3::from_array(pose.position) + delta).to_array();
        next.validate().ok()?;
        self.velocity += (target - self.velocity) * response;
        if target == DVec3::ZERO && self.velocity.length() < REST_SPEED {
            self.reset();
        }
        Some(next)
    }
}

impl CameraPose {
    /// Dolly translates along the optical axis without changing orientation or field of view.
    pub fn dollied(self, distance: f64) -> Option<Self> {
        if !distance.is_finite() {
            return None;
        }
        let direction = CameraProjection::new(self, 1.0)?.movement([0.0, 0.0, 1.0]);
        let next = Self {
            position: (DVec3::from_array(self.position) + direction * distance).to_array(),
            ..self
        };
        next.validate().ok()?;
        Some(next)
    }

    /// Equal wheel steps scale optical magnification, retaining the existing admitted FOV range.
    pub fn zoomed(self, steps: f64) -> Option<Self> {
        self.validate().ok()?;
        if !steps.is_finite() {
            return None;
        }
        let half = (self.fov.to_radians() * 0.5).tan().ln() - steps * 0.08;
        let low = (CAMERA_FOV_RANGE.start.to_radians() * 0.5).tan().ln();
        let high = ((CAMERA_FOV_RANGE.end - 1e-6).to_radians() * 0.5)
            .tan()
            .ln();
        let next = Self {
            fov: 2.0 * half.clamp(low, high).exp().atan().to_degrees(),
            ..self
        };
        next.validate().ok()?;
        Some(next)
    }

    /// Grabs the frozen view plane at the supplied pivot; the scene follows the pointer.
    pub fn panned(self, pivot: [f64; 3], aspect: f64, delta: [f64; 2]) -> Option<Self> {
        let projection = CameraProjection::new(self, aspect)?;
        let shifted = projection.dragged(pivot, delta)?;
        let next = Self {
            position: std::array::from_fn(|axis| {
                self.position[axis] - (shifted[axis] - pivot[axis])
            }),
            ..self
        };
        next.validate().ok()?;
        Some(next)
    }

    /// Leveling changes roll only; it neither resets the timeline view nor reframes a subject.
    pub fn leveled(self) -> Self {
        Self { roll: 0.0, ..self }
    }
}

#[cfg(test)]
mod tests;
