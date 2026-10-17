//! Rain and snow streaks from Cinnabar's precipitation layers (`PrecipitationScene`), placed
//! as `weather.wesl` places them: each layer wraps the shared particle mesh in a 30-block box
//! ahead of the camera and stretches every particle along its velocity. Vanilla's streak
//! width is a constant `dimensions.x / 1.778` blocks across the view; the vanilla fragment
//! shader's column occlusion is applied here per streak, cutting it at the column's surface.

use super::vertices::{Mesh, Vertex};
use crate::weather_render::PARTICLE_MESH_SEED;
use bevy::math::Vec3;
use render_model::{
    OCCLUSION_BLOCKED, OCCLUSION_SIDE, OcclusionGrid, PARTICLE_BOX, PARTICLE_POOL,
    PrecipitationLayerRecord, particle_mesh,
};
use std::sync::LazyLock;

/// Projection y-scale the vanilla widths are authored against (`weather.wesl`).
const REFERENCE_PROJECTION_SCALE: f32 = 1.778_341;
/// Weather is lit as an open-sky column: sky 15, block 0.
const LIGHT: [f32; 2] = [0.0, 240.0];

static PARTICLES: LazyLock<Vec<[f32; 4]>> = LazyLock::new(|| particle_mesh(PARTICLE_MESH_SEED));

fn wrap_box(value: Vec3) -> Vec3 {
    value - PARTICLE_BOX * (value / PARTICLE_BOX).trunc()
}

/// The height below which `kind` does not fall at world column (`x`, `z`), or `None` when it
/// falls at any height; `Some(i32::MAX)` where it never shows.
fn surface(grid: &OcclusionGrid, kind: u32, position: Vec3) -> Option<i32> {
    let cell = [
        position.x.floor() as i32 - grid.origin[0],
        position.z.floor() as i32 - grid.origin[1],
    ];
    if cell.iter().any(|c| !(0..OCCLUSION_SIDE).contains(c)) {
        return None;
    }
    let plane = (OCCLUSION_SIDE * OCCLUSION_SIDE) as usize;
    let index = kind as usize * plane + (cell[1] * OCCLUSION_SIDE + cell[0]) as usize;
    grid.heights.get(index).copied()
}

/// Appends every layer's streaks relative to `camera`.
pub(super) fn weather_quads(
    mesh: &mut Mesh,
    layers: &[PrecipitationLayerRecord],
    forward_offset: [f32; 3],
    grid: &OcclusionGrid,
    camera: Vec3,
) {
    let particles = PARTICLES.as_slice();
    if particles.is_empty() {
        return;
    }
    let forward = Vec3::from_array(forward_offset);
    for layer in layers {
        let base = Vec3::from_slice(&layer.base_offset[..3]);
        let velocity = Vec3::from_slice(&layer.velocity[..3]);
        let [u0, v0, du, dv] = layer.uv_rect;
        let [width, length] = layer.dimensions;
        let half = width / REFERENCE_PROJECTION_SCALE * 0.5;
        for quad in 0..layer.particle_count {
            let index = ((layer.first_particle + quad) % PARTICLE_POOL) as usize;
            let Some(particle) = particles.get(index) else {
                continue;
            };
            let relative = wrap_box(Vec3::new(particle[0], particle[1], particle[2]) + base)
                - Vec3::splat(PARTICLE_BOX * 0.5)
                + forward;
            // `head` is the particle (`v` 1), `tail` its trail along the velocity (`v` 0).
            let mut head = relative;
            let mut tail = relative + velocity * length;
            let (mut head_v, mut tail_v) = (1.0, 0.0);
            let lower = if head.y < tail.y { head } else { tail };
            match surface(grid, layer.kind, lower + camera) {
                Some(OCCLUSION_BLOCKED) => continue,
                Some(height) => {
                    let floor = height as f32 + 0.5 - camera.y;
                    if head.y < floor && tail.y < floor {
                        continue;
                    }
                    let t = |from: Vec3, to: Vec3| (floor - from.y) / (to.y - from.y);
                    if head.y < floor {
                        let s = t(head, tail);
                        head = head.lerp(tail, s);
                        head_v = 1.0 - s;
                    } else if tail.y < floor {
                        let s = t(tail, head);
                        tail = tail.lerp(head, s);
                        tail_v = s;
                    }
                }
                None => {}
            }
            let across = (tail - head).cross(head).normalize_or(Vec3::X) * half;
            let u = u0 + particle[3] * du;
            let normal = -head.normalize_or(Vec3::Z);
            let corner = |position: Vec3, x: f32, y: f32| Vertex {
                position,
                colour: [1.0; 4],
                normal,
                uv: [u + x * du, v0 + y * dv],
                mid_uv: [u + 0.5 * du, v0 + 0.5 * dv],
                light: LIGHT,
            };
            mesh.quad([
                corner(tail + across, 0.0, tail_v),
                corner(tail - across, 1.0, tail_v),
                corner(head - across, 1.0, head_v),
                corner(head + across, 0.0, head_v),
            ]);
        }
    }
}
