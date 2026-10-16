//! Placement and visual clock for a spawner's displayed actor.
use glam::{Mat4, Vec3};

const SPIN_NUMERATOR: f32 = 1000.0;
const SPIN_DELAY_OFFSET: f32 = 200.0;
const SPIN_DEGREES_PER_UNIT: f32 = 10.0;
const SPIN_WRAP_UNITS: i32 = 360;
const DISPLAY_CENTER_Y: f32 = 0.4;
const MINIMUM_RENDER_EXTENT: f32 = 0.1;
const HEIGHT_DIVISOR: f32 = 20.0;
const DISPLAY_TILT_RADIANS: f32 = -std::f32::consts::PI / 6.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct SpawnerDisplayClock {
    pub previous: f32,
    pub current: f32,
    pub delay: i32,
}

impl SpawnerDisplayClock {
    /// Advances integer spin endpoints and the client countdown only while activated.
    pub fn advance(&mut self, ticks: u32, activated: bool) {
        for _ in 0..ticks {
            self.previous = self.current;
            if activated {
                let increment = SPIN_NUMERATOR / (self.delay.max(-1) as f32 + SPIN_DELAY_OFFSET);
                let next = (self.current + increment) as i32;
                self.current = (next % SPIN_WRAP_UNITS) as f32;
                // Shift the old endpoint too, so wrapping preserves the forward interval.
                self.previous -= (next - next % SPIN_WRAP_UNITS) as f32;
                if self.delay > 0 {
                    self.delay -= 1;
                }
            }
        }
    }
}

/// Places a normal actor inside the cage using its rendered dimensions and sampled spin.
pub fn spawner_actor_transform(
    block: [i32; 3],
    dimensions: [f32; 3],
    spin: SpawnerDisplayClock,
    alpha: f32,
) -> Option<Mat4> {
    if !alpha.is_finite()
        || dimensions
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
    {
        return None;
    }
    let extent = dimensions.into_iter().fold(MINIMUM_RENDER_EXTENT, f32::max);
    let scale = extent.recip().min(1.0);
    let center = Vec3::new(
        block[0] as f32 + 0.5,
        block[1] as f32 + dimensions[1] / HEIGHT_DIVISOR + DISPLAY_CENTER_Y,
        block[2] as f32 + 0.5,
    );
    let angle = (spin.previous + (spin.current - spin.previous) * alpha.clamp(0.0, 1.0))
        * SPIN_DEGREES_PER_UNIT;
    Some(
        Mat4::from_translation(center)
            * Mat4::from_rotation_y(angle.to_radians())
            * Mat4::from_rotation_x(DISPLAY_TILT_RADIANS)
            * Mat4::from_translation(Vec3::new(0.0, -DISPLAY_CENTER_Y, 0.0))
            * Mat4::from_scale(Vec3::splat(scale)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn countdown_changes_spin_and_inactive_spawners_hold_their_pose() {
        let mut clock = SpawnerDisplayClock {
            delay: 2,
            ..Default::default()
        };
        clock.advance(2, true);
        assert_eq!((clock.previous, clock.current, clock.delay), (4.0, 8.0, 0));
        clock.advance(1, false);
        assert_eq!((clock.previous, clock.current, clock.delay), (8.0, 8.0, 0));
        clock.advance(1, true);
        assert_eq!(clock.current, 13.0);
    }
    #[test]
    fn regression_wrapped_spin_interpolates_forward_and_keeps_bounded_endpoints() {
        let mut clock = SpawnerDisplayClock {
            previous: 350.0,
            current: 355.0,
            delay: 0,
        };
        for _ in 0..145 {
            let start = clock.current;
            clock.advance(1, true);
            assert_eq!(clock.current - clock.previous, 5.0);
            assert!(clock.current >= 0.0 && clock.current < SPIN_WRAP_UNITS as f32);
            for alpha in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let expected = SpawnerDisplayClock {
                    previous: start,
                    current: start + 5.0,
                    delay: 0,
                };
                let actual = spawner_actor_transform([0; 3], [1.0; 3], clock, alpha).unwrap();
                let expected = spawner_actor_transform([0; 3], [1.0; 3], expected, alpha).unwrap();
                assert!(
                    actual.abs_diff_eq(expected, 0.0001),
                    "forward spin at alpha {alpha}"
                );
            }
        }
        clock.advance(1, false);
        assert_eq!(clock.previous, clock.current);
    }
    #[test]
    fn large_display_actors_fit_the_cage_and_keep_tilted_axes() {
        let small =
            spawner_actor_transform([0; 3], [0.6, 1.0, 0.6], Default::default(), 0.0).unwrap();
        let large =
            spawner_actor_transform([0; 3], [1.0, 4.0, 1.0], Default::default(), 0.0).unwrap();
        assert!((small.y_axis.truncate().length() - 1.0).abs() < 0.00001);
        assert!((large.y_axis.truncate().length() - 0.25).abs() < 0.00001);
        assert!(large.y_axis.z < 0.0);
    }
}
