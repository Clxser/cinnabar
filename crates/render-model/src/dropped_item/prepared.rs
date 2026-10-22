//! Camera-independent inputs for already received native rope geometry.

use super::{ItemMeshVertex, MAX_DYNAMIC_ITEM_VERTICES, rope_ribbon};
use bytemuck::{Pod, Zeroable};
use std::sync::Arc;

/// Maximum received terrain publications retained for recording item visibility.
pub const MAX_ITEM_TERRAIN_PUBLICATIONS: usize = 65_536;

#[derive(Clone, Default)]
pub struct ItemTerrainSnapshot {
    pub serial: u64,
    pub session_id: Option<u64>,
    pub complete: bool,
    /// Sorted native publication generations for the current item's terrain fence keys.
    pub generations: Arc<[(world::SubChunkKey, u64)]>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct PreparedItemRope {
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub segments: u32,
    pub sag: f32,
    pub half_width: f32,
    pub color: u32,
}
pub const PREPARED_ITEM_ROPE_WORDS: usize = std::mem::size_of::<PreparedItemRope>() / 4;

impl PreparedItemRope {
    /// Validates every source coordinate and bounds tessellation before admitting recorded data.
    pub fn from_words(words: [u32; PREPARED_ITEM_ROPE_WORDS]) -> Option<Self> {
        let rope: Self = bytemuck::cast(words);
        ((1..=MAX_DYNAMIC_ITEM_VERTICES / 6).contains(&(rope.segments as usize))
            && rope
                .from
                .iter()
                .chain(&rope.to)
                .chain(&[rope.sag, rope.half_width])
                .all(|v| v.is_finite())
            && rope.half_width >= 0.0)
            .then_some(rope)
    }

    /// Keeps the native curve, width and color unchanged for edited replay cameras.
    pub fn append(self, camera: [f32; 3], output: &mut Vec<ItemMeshVertex>) {
        if output.len() >= MAX_DYNAMIC_ITEM_VERTICES / 3 * 3 {
            return;
        }
        rope_ribbon(
            self.from,
            self.to,
            camera,
            self.segments as usize,
            self.sag,
            self.half_width,
            self.color,
            output,
        );
        output.truncate(MAX_DYNAMIC_ITEM_VERTICES / 3 * 3);
    }

    /// Serializes the explicit owned curve contract without inferring it from old triangles.
    pub fn words(self) -> [u32; PREPARED_ITEM_ROPE_WORDS] {
        bytemuck::cast(self)
    }
}
