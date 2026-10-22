//! Immutable text ink shared by worker rasterization and GPU overlay composition.

use std::sync::Arc;

/// Bounds a single prepared title independently of the output frame's allocation budget.
pub const MAX_TEXT_BITMAP_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TEXT_CACHE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_TEXT_LAYERS: usize = 128;

pub struct TextBitmap {
    size: [u32; 2],
    pixels: Arc<[u8]>,
}

impl TextBitmap {
    /// Admits tightly packed RGBA8. RGB is premultiplied in linear light, then sRGB encoded;
    /// alpha is linear coverage. A filtering sRGB texture and premultiplied blend preserve ink edges.
    pub fn new(size: [u32; 2], pixels: Arc<[u8]>) -> Result<Self, String> {
        let bytes = (size[0] as usize)
            .checked_mul(size[1] as usize)
            .and_then(|pixels| pixels.checked_mul(4));
        if size.contains(&0) || bytes != Some(pixels.len()) || pixels.len() > MAX_TEXT_BITMAP_BYTES
        {
            return Err("Title ink exceeds its bitmap budget or has invalid pixel storage.".into());
        }
        Ok(Self { size, pixels })
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}
