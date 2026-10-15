//! Bounded raster-layer composition in linear light; no UI or live-world state enters this owner.

use std::sync::{Arc, OnceLock};

use crate::{decode::DecodedFrame, model::Framing};

const MAX_PIXELS: usize = crate::video::MAX_FRAME_BYTES / std::mem::size_of::<[f32; 4]>();
const ENCODE_LEVELS: usize = 4096;
const SRGB_GAMMA: f32 = 2.4;
const SRGB_LINEAR_SLOPE: f32 = 12.92;

pub(super) struct Raster {
    size: [u32; 2],
    linear: Vec<[f32; 4]>,
}

impl Raster {
    /// The output is opaque black under every visual layer, including timeline gaps.
    pub fn new(size: [u32; 2]) -> Result<Self, String> {
        if size.contains(&0) {
            return Err("Preview dimensions must be nonzero.".into());
        }
        let pixels = (size[0] as usize)
            .checked_mul(size[1] as usize)
            .filter(|count| *count <= MAX_PIXELS)
            .ok_or("Preview composition exceeded its pixel budget.")?;
        Ok(Self {
            size,
            linear: vec![[0.0, 0.0, 0.0, 1.0]; pixels],
        })
    }

    /// Dissolves accumulate two weighted premultiplied sources before normal track compositing.
    pub fn mix(size: [u32; 2]) -> Result<Self, String> {
        let mut raster = Self::new(size)?;
        raster.linear.fill([0.0; 4]);
        Ok(raster)
    }

    /// Fits the authored crop inside the output and blends source alpha and clip opacity in track order.
    pub fn overlay(
        &mut self,
        source: &DecodedFrame,
        framing: Framing,
        weighted: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let [left, top, right, bottom] = framing.crop;
        if ![left, top, right, bottom, framing.opacity]
            .iter()
            .all(|value| value.is_finite())
            || left < 0.0
            || top < 0.0
            || right > 1.0
            || bottom > 1.0
            || left >= right
            || top >= bottom
            || !(0.0..=1.0).contains(&framing.opacity)
        {
            return Err("Preview crop or opacity is invalid.".into());
        }
        let [width, height] = source.size;
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4));
        if width == 0 || height == 0 || expected != Some(source.rgba8.len()) {
            return Err("Preview source pixels do not match their dimensions.".into());
        }
        let crop_width = f64::from(width) * (right - left);
        let crop_height = f64::from(height) * (bottom - top);
        let scale =
            (f64::from(self.size[0]) / crop_width).min(f64::from(self.size[1]) / crop_height);
        let draw_width = crop_width * scale;
        let draw_height = crop_height * scale;
        let x0 = (f64::from(self.size[0]) - draw_width) * 0.5;
        let y0 = (f64::from(self.size[1]) - draw_height) * 0.5;
        let linear = srgb_table();
        for y in y0.ceil().max(0.0) as u32..((y0 + draw_height).ceil() as u32).min(self.size[1]) {
            if cancelled() {
                return Err("Preview composition cancelled.".into());
            }
            let sy = (top * f64::from(height) + (f64::from(y) + 0.5 - y0) / scale)
                .floor()
                .clamp(0.0, f64::from(height - 1)) as usize;
            for x in x0.ceil().max(0.0) as u32..((x0 + draw_width).ceil() as u32).min(self.size[0])
            {
                let sx = (left * f64::from(width) + (f64::from(x) + 0.5 - x0) / scale)
                    .floor()
                    .clamp(0.0, f64::from(width - 1)) as usize;
                let pixel = &source.rgba8[(sy * width as usize + sx) * 4..][..4];
                let alpha = (f64::from(pixel[3]) / 255.0 * framing.opacity) as f32;
                let destination = &mut self.linear[(y * self.size[0] + x) as usize];
                let retained = if weighted { 1.0 } else { 1.0 - alpha };
                for channel in 0..3 {
                    destination[channel] =
                        destination[channel] * retained + linear[pixel[channel] as usize] * alpha;
                }
                destination[3] = destination[3] * retained + alpha;
            }
        }
        Ok(())
    }

    /// A dissolve is one layer over its background, avoiding the double darkening of two ordinary alpha-over draws.
    pub fn overlay_mix(
        &mut self,
        layer: Self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        if layer.size != self.size {
            return Err("Dissolve preview dimensions changed.".into());
        }
        for (row, source) in self
            .linear
            .chunks_mut(self.size[0] as usize)
            .zip(layer.linear.chunks(self.size[0] as usize))
        {
            if cancelled() {
                return Err("Preview composition cancelled.".into());
            }
            for (destination, source) in row.iter_mut().zip(source) {
                let retained = 1.0 - source[3].clamp(0.0, 1.0);
                for channel in 0..4 {
                    destination[channel] = destination[channel] * retained + source[channel];
                }
            }
        }
        Ok(())
    }

    /// Encodes opaque sRGB bytes for the UI media page; conversion and allocation remain on the worker.
    pub fn finish(self, cancelled: &impl Fn() -> bool) -> Result<Arc<[u8]>, String> {
        let mut bytes = Vec::with_capacity(self.linear.len() * 4);
        let encoded = encoded_table();
        for row in self.linear.chunks(self.size[0] as usize) {
            if cancelled() {
                return Err("Preview composition cancelled.".into());
            }
            for pixel in row {
                for channel in &pixel[..3] {
                    bytes.push(
                        encoded[(channel.clamp(0.0, 1.0) * (encoded.len() - 1) as f32).round()
                            as usize],
                    );
                }
                bytes.push(255);
            }
        }
        Ok(bytes.into())
    }
}

/// IEC sRGB decoding is tabulated once, avoiding a power operation for each source channel and frame.
fn srgb_table() -> &'static [f32; 256] {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|index| {
            let value = index as f32 / 255.0;
            if value <= 0.04045 {
                value / SRGB_LINEAR_SLOPE
            } else {
                ((value + 0.055) / 1.055).powf(SRGB_GAMMA)
            }
        })
    })
}

/// A fixed linear-light table bounds output quantization while retaining one source for the transfer curve.
fn encoded_table() -> &'static [u8; ENCODE_LEVELS] {
    static TABLE: OnceLock<[u8; ENCODE_LEVELS]> = OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|index| {
            let value = index as f32 / (ENCODE_LEVELS - 1) as f32;
            let encoded = if value <= 0.0031308 {
                value * SRGB_LINEAR_SLOPE
            } else {
                1.055 * value.powf(1.0 / SRGB_GAMMA) - 0.055
            };
            (encoded * 255.0).round().clamp(0.0, 255.0) as u8
        })
    })
}
