//! Worker-only rasterization reuses the installed OreUI faces, layout and coverage interpretation.

use std::sync::Arc;

use assets::{FontRendering, FontTexturePage, RuntimeFontCatalog};
use recording_studio::model::TitleStyle;
use render_model::{
    font_shading,
    text_bitmap::{MAX_TEXT_BITMAP_BYTES, TextBitmap},
};
use ui::{GlyphQuad, TextLayoutCache, TextLayoutRequest, TextStyle, TextWrap, UiScale};

#[derive(Clone, PartialEq)]
pub(super) struct Key {
    pub text: String,
    pub width: u32,
    pub pixels: u32,
    pub style: TitleStyle,
    pub rgb: [u8; 3],
    pub shadow: bool,
}

/// Lays out bounded multiline ink, preserving the font's SDF scale, gamma, formatting and overhangs.
pub(super) fn raster(
    font: &RuntimeFontCatalog,
    layouts: &mut TextLayoutCache,
    key: &Key,
    cancelled: &impl Fn() -> bool,
) -> Result<Arc<TextBitmap>, String> {
    let face = match key.style {
        TitleStyle::Body => ui::oreui_theme::OreUiFont::Seven,
        TitleStyle::Heading => ui::oreui_theme::OreUiFont::Ten,
    };
    let font = font.font_named_at_size(face.name(), key.pixels as f32);
    let (em, line, baseline) = match font.line_metrics() {
        Some(metrics) => (
            metrics.em_64,
            (metrics.em_64 as f32 * 1.2).round() as u32,
            metrics.ascent_64,
        ),
        None => (
            ui::FONT_INK_TEXELS * 64,
            ui::TEXT_LINE_HEIGHT_64,
            ui::TEXT_BASELINE_64,
        ),
    };
    let wanted = key.pixels as f32 * 64.0 / em.max(1) as f32;
    let scale = UiScale::new_display(wanted.clamp(UiScale::DISPLAY_MIN, UiScale::DISPLAY_MAX))
        .map_err(|e| format!("Title scale: {e:?}"))?;
    let multiplier = wanted / scale.get();
    let layout = layouts
        .layout(TextLayoutRequest {
            text: &key.text,
            style: TextStyle::default(),
            width_64: (key.width as f32 * 64.0 / multiplier).round().max(1.0) as u32,
            line_height_64: line,
            baseline_64: baseline,
            scale,
            font,
            wrap: TextWrap {
                align: ui::TextLineAlign::Center,
                max_lines: Some(32),
                snap_glyphs_to_device_pixels: font.rendering() == FontRendering::Coverage
                    && !font.linear_sampling(),
                ..Default::default()
            },
        })
        .map_err(|e| format!("Title layout failed: {e:?}"))?;
    if layout.ellipsized() {
        return Err("This title has too many lines. Shorten it or reduce its size.".into());
    }
    let effect_scale = layout.key().effect_scale() * multiplier;
    let bold = ui::TEXT_BOLD_OFFSET_64 as f32 / 64.0 * effect_scale;
    let italic = ui::TEXT_ITALIC_SHEAR_PX * effect_scale;
    let shadow = if key.shadow {
        ui::TEXT_SHADOW_OFFSET_64 as f32 / 64.0 * effect_scale
    } else {
        0.0
    };
    let mut ink = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for glyph in layout.glyphs() {
        if cancelled() {
            return Err("Title preparation cancelled.".into());
        }
        let bounds = glyph.bounds_64.map(|v| v as f32 / 64.0 * multiplier);
        if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
            continue;
        }
        ink[0] = ink[0].min(bounds[0]);
        ink[1] = ink[1].min(bounds[1]);
        ink[2] = ink[2].max(
            bounds[2]
                + if glyph.style.bold { bold } else { 0.0 }
                + if glyph.style.italic { italic } else { 0.0 }
                + shadow,
        );
        ink[3] = ink[3].max(bounds[3] + shadow);
    }
    if !ink.iter().all(|v| v.is_finite()) {
        return Ok(Arc::new(TextBitmap::new([1, 1], vec![0; 4].into())?));
    }
    let origin = [ink[0].floor() - 1.0, ink[1].floor() - 1.0];
    let size = [
        (ink[2].ceil() - origin[0] + 1.0) as u32,
        (ink[3].ceil() - origin[1] + 1.0) as u32,
    ];
    let count = (size[0] as usize)
        .checked_mul(size[1] as usize)
        .filter(|&n| n <= MAX_TEXT_BITMAP_BYTES / 4)
        .ok_or("This title exceeds the ink budget. Reduce its size or split the text.")?;
    // At most 64 MiB of float scratch, released before this immutable bitmap is cached.
    let mut pixels = vec![[0.0f32; 4]; count];
    let palette = ui::FormattingPalette::default();
    for is_shadow in [true, false] {
        if is_shadow && !key.shadow {
            continue;
        }
        for glyph in layout.glyphs() {
            if cancelled() {
                return Err("Title preparation cancelled.".into());
            }
            let page = font
                .pages()
                .get(glyph.page as usize)
                .ok_or("A title glyph refers to a missing font page.")?;
            let formatted = palette.rgb(glyph.style.color).unwrap_or([255; 3]);
            let tint = std::array::from_fn(|i| {
                let value = f32::from(formatted[i]) / 255.0 * f32::from(key.rgb[i]) / 255.0;
                if is_shadow { value * 0.25 } else { value }
            });
            let offset = if is_shadow { shadow } else { 0.0 };
            let bounds = glyph.bounds_64.map(|v| v as f32 / 64.0 * multiplier);
            for dx in [Some(0.0), glyph.style.bold.then_some(bold)]
                .into_iter()
                .flatten()
            {
                let bounds = [
                    bounds[0] - origin[0] + dx + offset,
                    bounds[1] - origin[1] + offset,
                    bounds[2] - origin[0] + dx + offset,
                    bounds[3] - origin[1] + offset,
                ];
                draw(
                    &mut pixels,
                    size,
                    page,
                    glyph,
                    bounds,
                    if glyph.style.italic { italic } else { 0.0 },
                    tint,
                    cancelled,
                )?;
            }
        }
    }
    let mut rgba = Vec::with_capacity(count * 4);
    for (index, pixel) in pixels.into_iter().enumerate() {
        if index % size[0] as usize == 0 && cancelled() {
            return Err("Title preparation cancelled.".into());
        }
        rgba.extend_from_slice(&[
            encode(pixel[0]),
            encode(pixel[1]),
            encode(pixel[2]),
            (pixel[3].clamp(0.0, 1.0) * 255.0).round() as u8,
        ]);
    }
    Ok(Arc::new(TextBitmap::new(size, rgba.into())?))
}

/// Samples only the glyph's exclusive UV rectangle, preventing filtering from neighbouring atlas glyphs.
#[allow(clippy::too_many_arguments)]
fn draw(
    pixels: &mut [[f32; 4]],
    size: [u32; 2],
    page: &FontTexturePage,
    glyph: &GlyphQuad,
    bounds: [f32; 4],
    shear: f32,
    tint: [f32; 3],
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    let [u0, v0, u1, v1] = glyph.uv.map(f32::from);
    let (width, height) = (bounds[2] - bounds[0], bounds[3] - bounds[1]);
    if u1 <= u0 || v1 <= v0 || width <= 0.0 || height <= 0.0 {
        return Ok(());
    }
    let scale = ((u1 - u0) * (v1 - v0) / (width * height)).sqrt();
    for y in bounds[1].floor().max(0.0) as u32..(bounds[3].ceil() as u32).min(size[1]) {
        if cancelled() {
            return Err("Title preparation cancelled.".into());
        }
        let fy = (y as f32 + 0.5 - bounds[1]) / height;
        if !(0.0..1.0).contains(&fy) {
            continue;
        }
        let shift = shear * (1.0 - fy);
        for x in (bounds[0] + shift).floor().max(0.0) as u32
            ..((bounds[2] + shift).ceil() as u32).min(size[0])
        {
            let fx = (x as f32 + 0.5 - bounds[0] - shift) / width;
            if !(0.0..1.0).contains(&fx) {
                continue;
            }
            let uv = [u0 + fx * (u1 - u0), v0 + fy * (v1 - v0)];
            let linear = glyph.rendering == FontRendering::NativeSdf
                || (glyph.rendering == FontRendering::Coverage && glyph.linear_sampling);
            let mut sample = sample(page, glyph.uv, uv, linear);
            if glyph.rendering == FontRendering::NativeSdf {
                sample = [1.0, 1.0, 1.0, font_shading::sdf_coverage(sample[3], scale)];
            }
            if glyph.rendering != FontRendering::Coverage {
                sample = [1.0, 1.0, 1.0, font_shading::coverage_gamma(sample[3], tint)];
            }
            let alpha = sample[3];
            let target = &mut pixels[(y * size[0] + x) as usize];
            for c in 0..3 {
                target[c] = decode(sample[c] * tint[c]) * alpha + target[c] * (1.0 - alpha);
            }
            target[3] = alpha + target[3] * (1.0 - alpha);
        }
    }
    Ok(())
}

/// Font coverage is linear data; bilinear filtering never decodes it as scene color.
fn sample(page: &FontTexturePage, rect: [u16; 4], uv: [f32; 2], linear: bool) -> [f32; 4] {
    let texel = |x: i32, y: i32| {
        let x = x.clamp(i32::from(rect[0]), i32::from(rect[2]) - 1);
        let y = y.clamp(i32::from(rect[1]), i32::from(rect[3]) - 1);
        if x as u32 >= page.width || y as u32 >= page.height {
            return [0.0; 4];
        }
        page.pixels
            .texel((y as u32 * page.width + x as u32) as usize)
            .unwrap_or([0; 4])
            .map(|v| f32::from(v) / 255.0)
    };
    if !linear {
        return texel(uv[0].floor() as i32, uv[1].floor() as i32);
    }
    let [x, y] = uv.map(|v| v - 0.5);
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let [a, b, c, d] = [
        texel(ix, iy),
        texel(ix + 1, iy),
        texel(ix, iy + 1),
        texel(ix + 1, iy + 1),
    ];
    std::array::from_fn(|i| {
        (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy
    })
}

/// Converts straight encoded font color to the compositing space.
fn decode(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}
/// Encodes premultiplied linear ink; the GPU sRGB sampler recovers the premultiplied value.
fn encode(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    ((if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }) * 255.0)
        .round() as u8
}
