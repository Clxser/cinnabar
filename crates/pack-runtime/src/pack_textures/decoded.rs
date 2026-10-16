//! Strip frame operations on decoded source pixels.

use super::DecodedTexture;

impl DecodedTexture {
    /// Keeps the upper-left square frame of an integral strip on either axis.
    /// Square images, other rectangles, and malformed pixel buffers remain unchanged.
    pub fn retain_first_strip_frame(&mut self) {
        let side = self.width.min(self.height);
        let extent = self.width.max(self.height);
        if side == 0
            || side == extent
            || !extent.is_multiple_of(side)
            || self
                .width
                .checked_mul(self.height)
                .and_then(|pixels| pixels.checked_mul(4))
                .and_then(|bytes| usize::try_from(bytes).ok())
                != Some(self.rgba8.len())
        {
            return;
        }
        let frame_bytes = (side * side * 4) as usize;
        if self.width == side {
            let mut pixels = std::mem::take(&mut self.rgba8).into_vec();
            pixels.truncate(frame_bytes);
            self.rgba8 = pixels.into_boxed_slice();
        } else {
            let mut pixels = Vec::with_capacity(frame_bytes);
            for row in self.rgba8.chunks_exact(self.width as usize * 4) {
                pixels.extend_from_slice(&row[..side as usize * 4]);
            }
            self.rgba8 = pixels.into_boxed_slice();
        }
        self.width = side;
        self.height = side;
    }
}
