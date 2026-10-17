//! Camera-independent prepared samples from particles already admitted by the native simulation.

use crate::{DrawLists, ParticleInstance, ParticleView, def::Facing};
use bytemuck::{Pod, Zeroable};

/// Resolved material, tint, UV, light and billboard inputs; no emitter or Molang authority is replayed.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct PreparedParticle {
    pub instance: ParticleInstance,
    pub emitter_basis: [[f32; 3]; 3],
    /// XYZ direction, W is one when an admitted direction exists.
    pub direction: [f32; 4],
    /// Billboard width, height, rotation in degrees, reserved zero.
    pub size_rotation: [f32; 4],
    pub facing: u32,
    /// Opaque/cutout, alpha-over, additive.
    pub material: u32,
}
pub const PREPARED_PARTICLE_WORDS: usize = std::mem::size_of::<PreparedParticle>() / 4;

impl PreparedParticle {
    /// Every enum selector and numerical shader input is checked before a recording becomes drawable.
    pub fn from_words(words: [u32; PREPARED_PARTICLE_WORDS]) -> Option<Self> {
        let value: Self = bytemuck::cast(words);
        (Facing::from_recording(value.facing).is_some()
            && value.material <= 2
            && [0.0, 1.0].contains(&value.direction[3])
            && value.size_rotation[3] == 0.0
            && bytemuck::cast_slice::<ParticleInstance, f32>(&[value.instance])
                .iter()
                .all(|v| v.is_finite())
            && value
                .emitter_basis
                .iter()
                .flatten()
                .chain(&value.direction)
                .chain(&value.size_rotation)
                .all(|v| v.is_finite()))
        .then_some(value)
    }

    /// Encodes the owner's complete fixed contract without inferring a facing mode from old vertices.
    pub fn words(self) -> [u32; PREPARED_PARTICLE_WORDS] {
        bytemuck::cast(self)
    }

    /// Rebuilds camera-facing axes with the same native facing and rotation functions as live draws.
    pub fn sample(self, view: &ParticleView) -> ParticleInstance {
        let facing = Facing::from_recording(self.facing).expect("admitted prepared facing");
        let direction = (self.direction[3] != 0.0).then_some([
            self.direction[0],
            self.direction[1],
            self.direction[2],
        ]);
        let center = [
            self.instance.center_light[0],
            self.instance.center_light[1],
            self.instance.center_light[2],
        ];
        let (right, up) = crate::draw::billboard_axes(
            facing,
            view,
            &self.emitter_basis,
            center,
            direction,
            self.size_rotation[2],
        );
        let mut instance = self.instance;
        for axis in 0..3 {
            instance.axis_x[axis] = right[axis] * self.size_rotation[0];
            instance.axis_y[axis] = up[axis] * self.size_rotation[1];
        }
        instance
    }
}

/// Reprojects every received sample and applies the owner's distance/frustum and depth-sort policy.
pub fn draw(prepared: &[PreparedParticle], view: &ParticleView) -> DrawLists {
    let mut collected = Vec::with_capacity(prepared.len().min(crate::MAX_LIVE_PARTICLES));
    let cos_limit = (view.half_diagonal + 0.15).min(3.0).cos();
    for particle in prepared.iter().take(crate::MAX_LIVE_PARTICLES) {
        let p = particle.instance.center_light;
        let relative = [
            p[0] - view.position[0],
            p[1] - view.position[1],
            p[2] - view.position[2],
        ];
        let distance_sq = relative.iter().map(|v| v * v).sum::<f32>();
        let depth = (0..3)
            .map(|axis| relative[axis] * view.forward[axis])
            .sum::<f32>();
        if distance_sq > crate::draw::MAX_DRAW_DISTANCE.powi(2)
            || (distance_sq > 4.0 && depth < cos_limit * distance_sq.sqrt())
        {
            continue;
        }
        collected.push((depth, particle.material, particle.sample(view)));
    }
    collected.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
    let mut lists = DrawLists::default();
    for (_, material, instance) in collected {
        match material {
            0 => lists.opaque.push(instance),
            1 => lists.blend.push(instance),
            2 => lists.add.push(instance),
            _ => unreachable!("admitted particle material"),
        }
    }
    lists
}
