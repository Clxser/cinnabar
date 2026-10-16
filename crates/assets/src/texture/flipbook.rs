//! Source descriptions shared by carrier compilation and active pack overlays.

use serde::{Deserialize, Serialize};

/// Maximum number of explicit frame indices retained for one flipbook.
pub const MAX_FLIPBOOK_FRAMES: usize = 4_096;
/// Frame interpolation is enabled unless a description opts out.
pub const DEFAULT_BLEND_FRAMES: bool = true;

/// An animation source and the atlas destination it updates.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct FlipbookSource {
    #[serde(rename = "flipbook_texture")]
    pub texture_path: Box<str>,
    pub atlas_tile: Box<str>,
    #[serde(default = "one")]
    pub ticks_per_frame: u32,
    #[serde(default)]
    pub frames: Box<[u32]>,
    #[serde(default)]
    pub atlas_index: u32,
    #[serde(default)]
    pub atlas_tile_variant: u32,
    /// An omitted tile variant updates every UV variant at the selected atlas index.
    #[serde(default)]
    pub all_variants: bool,
    #[serde(default = "one")]
    pub replicate: u32,
    #[serde(default = "default_blend")]
    pub blend_frames: bool,
}

impl FlipbookSource {
    /// Rejects empty routes, zero timing/replication, and oversized or overflowing timelines.
    pub fn is_valid(&self) -> bool {
        !self.texture_path.is_empty()
            && !self.atlas_tile.is_empty()
            && self.ticks_per_frame > 0
            && self.replicate > 0
            && self.frames.len() <= MAX_FLIPBOOK_FRAMES
            && (self.frames.len().max(1) as u32)
                .checked_mul(self.ticks_per_frame)
                .is_some()
    }
}

/// Supplies the default positive timing and replication value.
fn one() -> u32 {
    1
}

/// Supplies the shared interpolation default for deserialization.
fn default_blend() -> bool {
    DEFAULT_BLEND_FRAMES
}
