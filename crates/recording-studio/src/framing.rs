//! One inverse image transform owns crop, aspect preservation and arbitrary rotation for every compositor.

use crate::{
    StudioError,
    model::{Framing, FramingFit},
};

pub const FULL_TURN_DEGREES: f64 = 360.0;

impl Framing {
    /// Serialized angles may contain any finite number of turns; sampling reduces them before trigonometry.
    pub fn validate(self) -> Result<(), StudioError> {
        let [left, top, right, bottom] = self.crop;
        if !self
            .crop
            .into_iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
            || left >= right
            || top >= bottom
            || !self.opacity.is_finite()
            || !(0.0..=1.0).contains(&self.opacity)
            || !self.rotation_degrees.is_finite()
        {
            return Err(StudioError::Invalid(
                "framing needs a nonempty normalized crop, normalized opacity and finite rotation",
            ));
        }
        Ok(())
    }
}

/// Rows map normalized canvas coordinates into the unrotated retained crop, with matching CPU/GPU precision.
#[derive(Clone, Copy, Debug)]
pub struct FrameTransform {
    pub inverse_x: [f32; 4],
    pub inverse_y: [f32; 4],
    pub crop: [f32; 4],
    /// Conservative clipped pixel bounds let the CPU skip the empty exterior of a fitted image.
    pub bounds: [u32; 4],
    low: [f32; 2],
    high: [f32; 2],
}

impl FrameTransform {
    /// Fill covers all inverse-rotated canvas corners; fitting the rotated bounding box alone leaves corner bars.
    pub fn new(framing: Framing, source: [u32; 2], output: [u32; 2]) -> Result<Self, String> {
        framing.validate().map_err(|error| error.to_string())?;
        if source.contains(&0) || output.contains(&0) {
            return Err("Image framing requires nonzero source and output dimensions.".into());
        }
        let [left, top, right, bottom] = framing.crop;
        let width = f64::from(source[0]) * (right - left);
        let height = f64::from(source[1]) * (bottom - top);
        let [canvas_w, canvas_h] = output.map(f64::from);
        let (sin, cos) = framing
            .rotation_degrees
            .rem_euclid(FULL_TURN_DEGREES)
            .to_radians()
            .sin_cos();
        let rotated_w = cos.abs() * width + sin.abs() * height;
        let rotated_h = sin.abs() * width + cos.abs() * height;
        let scale = match framing.fit {
            FramingFit::Fit => (canvas_w / rotated_w).min(canvas_h / rotated_h),
            FramingFit::Fill => ((cos.abs() * canvas_w + sin.abs() * canvas_h) / width)
                .max((sin.abs() * canvas_w + cos.abs() * canvas_h) / height),
        };
        let row = |x: f64, y: f64| [x as f32, y as f32, (0.5 - 0.5 * (x + y)) as f32, 0.0];
        let inverse_x = row(
            cos * canvas_w / (width * scale),
            sin * canvas_h / (width * scale),
        );
        let inverse_y = row(
            -sin * canvas_w / (height * scale),
            cos * canvas_h / (height * scale),
        );
        if !scale.is_finite()
            || scale <= 0.0
            || !inverse_x.into_iter().chain(inverse_y).all(f32::is_finite)
            || inverse_x[..2].iter().all(|value| *value == 0.0)
            || inverse_y[..2].iter().all(|value| *value == 0.0)
        {
            return Err("The image crop is too small for a finite composition transform.".into());
        }
        let crop = framing.crop.map(|value| value as f32);
        let half_texel = source.map(|side| 0.5 / side as f32);
        let middle = [(crop[0] + crop[2]) * 0.5, (crop[1] + crop[3]) * 0.5];
        let low = std::array::from_fn(|axis| (crop[axis] + half_texel[axis]).min(middle[axis]));
        let high =
            std::array::from_fn(|axis| (crop[axis + 2] - half_texel[axis]).max(middle[axis]));
        let draw_w = rotated_w * scale;
        let draw_h = rotated_h * scale;
        let bounds = [
            ((canvas_w - draw_w) * 0.5).floor().clamp(0.0, canvas_w) as u32,
            ((canvas_h - draw_h) * 0.5).floor().clamp(0.0, canvas_h) as u32,
            ((canvas_w + draw_w) * 0.5).ceil().clamp(0.0, canvas_w) as u32,
            ((canvas_h + draw_h) * 0.5).ceil().clamp(0.0, canvas_h) as u32,
        ];
        Ok(Self {
            inverse_x,
            inverse_y,
            crop,
            bounds,
            low,
            high,
        })
    }

    /// Out-of-crop coordinates are transparent; retained edge texels stay inside the authored crop while filtering.
    pub fn source_uv(&self, canvas: [f32; 2]) -> Option<[f32; 2]> {
        let evaluate = |row: [f32; 4]| row[0] * canvas[0] + row[1] * canvas[1] + row[2];
        let local = [evaluate(self.inverse_x), evaluate(self.inverse_y)];
        if local.into_iter().any(|value| !(0.0..=1.0).contains(&value)) {
            return None;
        }
        Some(std::array::from_fn(|axis| {
            (self.crop[axis] + (self.crop[axis + 2] - self.crop[axis]) * local[axis])
                .clamp(self.low[axis], self.high[axis])
        }))
    }
}

#[cfg(test)]
mod tests;
