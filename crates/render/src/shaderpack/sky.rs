//! The vanilla sky geometry `gbuffers_skybasic` draws: the sky disc above the camera, the dark
//! disc below it when the camera is under the horizon, and the sunrise/sunset fan, positioned
//! relative to the camera with their colours in the vertices (Iris's `gl_Color`).
//!
//! Geometry and colours follow Java's sky renderer: a 512-block disc 16 blocks above the
//! camera in eight 45° segments, the dark disc 4 blocks below, and a 16-segment fan whose
//! colour and height track the sun near the horizon.

use super::geometry::{IRIS_VERTEX_STRIDE, host_vertex};
use bevy::math::{Mat3, Vec3};
use std::ops::Range;

/// Most vertices one frame's sky uses: two eight-segment discs and a sixteen-segment fan.
pub(crate) const MAX_SKY_VERTICES: u32 = (8 + 8 + 16) * 3;
/// Bytes of the per-view sky vertex buffer.
pub(crate) const SKY_BUFFER_BYTES: u64 = MAX_SKY_VERTICES as u64 * IRIS_VERTEX_STRIDE;
/// Java's horizon height of a normal world: the dark disc shows below it.
const HORIZON_HEIGHT: f32 = 63.0;
const DISC_RADIUS: f32 = 512.0;
const LIGHT_DISC_HEIGHT: f32 = 16.0;
/// The dark disc sits at -16, drawn translated 12 blocks up.
const DARK_DISC_HEIGHT: f32 = -4.0;

/// One frame's sky: vertex bytes and the vertex ranges each draw covers.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SkyGeometry {
    pub bytes: Vec<u8>,
    /// The discs, drawn with the sky program.
    pub discs: Range<u32>,
    /// The sunrise/sunset fan, drawn blended; empty when the sun is not near the horizon.
    pub sunset: Range<u32>,
}

/// What the sky depends on this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SkyInputs {
    /// `skyColor`, gamma.
    pub sky_color: [f32; 3],
    /// Java's time of day, 0 at noon (`ShaderPackFrameInputs::celestial_angle`).
    pub celestial_angle: f32,
    /// The camera's world Y.
    pub camera_y: f32,
    /// The dimension has a sunrise colour (the Overworld).
    pub sunrise: bool,
}

/// Java's sunrise/sunset colour for time of day `angle`: RGB and the fan's opacity, or `None`
/// when the sun is far from the horizon.
pub(crate) fn sunrise_color(angle: f32) -> Option<[f32; 4]> {
    let height = (angle * std::f32::consts::TAU).cos();
    if !(-0.4..=0.4).contains(&height) {
        return None;
    }
    let i = height / 0.4 * 0.5 + 0.5;
    let alpha = 1.0 - (1.0 - (i * std::f32::consts::PI).sin()) * 0.99;
    Some([i * 0.3 + 0.7, i * i * 0.7 + 0.2, 0.2, alpha * alpha])
}

fn fan(out: &mut Vec<u8>, centre: ([f32; 3], [f32; 4]), ring: &[([f32; 3], [f32; 4])]) -> u32 {
    let mut push = |(position, colour): ([f32; 3], [f32; 4])| {
        for word in host_vertex(position, colour, [0.0, 1.0, 0.0]) {
            out.extend_from_slice(&word.to_le_bytes());
        }
    };
    for pair in ring.windows(2) {
        push(centre);
        push(pair[0]);
        push(pair[1]);
    }
    (ring.len().saturating_sub(1) * 3) as u32
}

fn disc(out: &mut Vec<u8>, height: f32, colour: [f32; 4]) -> u32 {
    let x_radius = height.signum() * DISC_RADIUS;
    let ring: Vec<_> = (-4..=4)
        .map(|step| {
            let angle = (step as f32 * 45.0).to_radians();
            (
                [x_radius * angle.cos(), height, DISC_RADIUS * angle.sin()],
                colour,
            )
        })
        .collect();
    fan(out, ([0.0, height, 0.0], colour), &ring)
}

/// Builds this frame's sky.
pub(crate) fn sky_geometry(inputs: SkyInputs) -> SkyGeometry {
    let mut bytes = Vec::with_capacity(SKY_BUFFER_BYTES as usize);
    let [r, g, b] = inputs.sky_color;
    let mut count = disc(&mut bytes, LIGHT_DISC_HEIGHT, [r, g, b, 1.0]);
    if inputs.camera_y < HORIZON_HEIGHT {
        count += disc(&mut bytes, DARK_DISC_HEIGHT, [0.0, 0.0, 0.0, 1.0]);
    }
    let discs = 0..count;
    let sunset_colour = inputs
        .sunrise
        .then(|| sunrise_color(inputs.celestial_angle))
        .flatten();
    if let Some([r, g, b, a]) = sunset_colour {
        let sun = (inputs.celestial_angle * std::f32::consts::TAU).sin();
        let flip = if sun < 0.0 { std::f32::consts::PI } else { 0.0 };
        let rotation = Mat3::from_rotation_x(std::f32::consts::FRAC_PI_2)
            * Mat3::from_rotation_z(flip)
            * Mat3::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let place = |v: Vec3| (rotation * v).to_array();
        let ring: Vec<_> = (0..=16)
            .map(|step| {
                let angle = step as f32 * std::f32::consts::TAU / 16.0;
                let (sin, cos) = angle.sin_cos();
                (
                    place(Vec3::new(sin * 120.0, cos * 120.0, -cos * 40.0 * a)),
                    [r, g, b, 0.0],
                )
            })
            .collect();
        count += fan(&mut bytes, (place(Vec3::Y * 100.0), [r, g, b, a]), &ring);
    }
    SkyGeometry {
        bytes,
        sunset: discs.end..count,
        discs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(celestial_angle: f32, camera_y: f32) -> SkyInputs {
        SkyInputs {
            sky_color: [0.5, 0.7, 1.0],
            celestial_angle,
            camera_y,
            sunrise: true,
        }
    }

    fn vertex_word(sky: &SkyGeometry, vertex: u32, word: usize) -> u32 {
        let at = vertex as usize * IRIS_VERTEX_STRIDE as usize + word * 4;
        u32::from_le_bytes(sky.bytes[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn noon_draws_only_the_sky_disc_above_the_camera() {
        let sky = sky_geometry(inputs(0.0, 80.0));
        assert_eq!(sky.discs, 0..24, "eight triangles");
        assert!(sky.sunset.is_empty());
        assert_eq!(sky.bytes.len(), 24 * IRIS_VERTEX_STRIDE as usize);
        for vertex in sky.discs.clone() {
            assert_eq!(f32::from_bits(vertex_word(&sky, vertex, 1)), 16.0);
        }
        assert_eq!(
            vertex_word(&sky, 0, 3),
            u32::from_le_bytes([128, 179, 255, 255])
        );
    }

    #[test]
    fn under_the_horizon_adds_the_dark_disc_and_sunset_adds_the_fan() {
        let sky = sky_geometry(inputs(0.25, 40.0));
        assert_eq!(sky.discs, 0..48);
        assert_eq!(sky.sunset, 48..96, "sixteen fan triangles at sunset");
        assert!(sky.bytes.len() as u64 <= SKY_BUFFER_BYTES);
        let colour = sunrise_color(0.25).expect("the sun sets at a quarter day");
        assert!(
            (colour[3] - 1.0).abs() < 1e-3,
            "fully opaque at the horizon"
        );
        assert_eq!(sunrise_color(0.0), None);
    }
}
