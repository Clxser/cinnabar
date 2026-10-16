//! Physical frame extraction and source image resampling.

use super::{DecodedTexture, FlipbookSource};
use std::collections::HashMap;

/// Unique physical pixels and the ordered frame indices that play them.
#[derive(Default)]
pub(in super::super) struct FrameSequence {
    pub(in super::super) frames: Vec<DecodedTexture>,
    pub(in super::super) timeline: Vec<u32>,
}

/// Cuts bounded square frames, reusing identical pixels throughout the authored timeline.
/// Crops replication and shrinks before charging unique frames against source capacity.
pub(in super::super) fn flipbook_frames(
    texture: &DecodedTexture,
    flipbook: &FlipbookSource,
    max_frames: usize,
    max_side: u32,
) -> FrameSequence {
    let side = texture.width.min(texture.height);
    if max_frames == 0 || side == 0 {
        return FrameSequence::default();
    }
    let long_side = texture.width.max(texture.height);
    if !long_side.is_multiple_of(side) {
        return FrameSequence::default();
    }
    let count = long_side / side;
    let horizontal = texture.width > texture.height;
    let crop = (side / flipbook.replicate.max(1)).max(1);
    let output = crop.min(max_side).max(1);
    let len = if flipbook.frames.is_empty() {
        count as usize
    } else {
        flipbook.frames.len()
    };
    let mut physical = HashMap::<DecodedTexture, u32>::new();
    let mut by_source_frame = HashMap::new();
    let mut timeline = Vec::new();
    for index in 0..len.min(assets::texture::flipbook::MAX_FLIPBOOK_FRAMES) {
        let frame = if flipbook.frames.is_empty() {
            index as u32
        } else {
            flipbook.frames[index]
        };
        let frame = frame.min(count - 1);
        if let Some(&physical_frame) = by_source_frame.get(&frame) {
            timeline.push(physical_frame);
            continue;
        }
        let start = frame * side;
        let (x_start, y_start) = if horizontal { (start, 0) } else { (0, start) };
        let mut rgba8 = Vec::with_capacity((output * output * 4) as usize);
        for y in 0..output {
            let source_y = y_start + y * crop / output;
            for x in 0..output {
                let source_x = x_start + x * crop / output;
                let start = ((source_y * texture.width + source_x) * 4) as usize;
                rgba8.extend_from_slice(&texture.rgba8[start..start + 4]);
            }
        }
        let pixels = DecodedTexture {
            width: output,
            height: output,
            rgba8: rgba8.into_boxed_slice(),
        };
        let next = physical.len() as u32;
        let physical_frame = match physical.entry(pixels) {
            std::collections::hash_map::Entry::Occupied(existing) => *existing.get(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                if next as usize >= max_frames {
                    break;
                }
                entry.insert(next);
                next
            }
        };
        by_source_frame.insert(frame, physical_frame);
        timeline.push(physical_frame);
    }
    let mut frames = physical.into_iter().collect::<Vec<_>>();
    frames.sort_unstable_by_key(|(_, index)| *index);
    FrameSequence {
        frames: frames.into_iter().map(|(pixels, _)| pixels).collect(),
        timeline,
    }
}

/// Downscales aspect-preserving so the longest side is at most `max_side`;
/// smaller images are returned unchanged. Nearest-neighbour keeps pixel art crisp.
#[cfg(test)]
pub(in super::super) fn shrink_to_max(texture: &DecodedTexture, max_side: u32) -> DecodedTexture {
    let longest = texture.width.max(texture.height);
    if longest <= max_side {
        return texture.clone();
    }
    let scale = |side: u32| (side * max_side / longest).max(1);
    let (width, height) = (scale(texture.width), scale(texture.height));
    let mut rgba8 = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        let source_y = (u64::from(y) * u64::from(texture.height) / u64::from(height)) as usize;
        for x in 0..width {
            let source_x = (u64::from(x) * u64::from(texture.width) / u64::from(width)) as usize;
            let offset = (source_y * texture.width as usize + source_x) * 4;
            rgba8.extend_from_slice(&texture.rgba8[offset..offset + 4]);
        }
    }
    DecodedTexture {
        width,
        height,
        rgba8: rgba8.into_boxed_slice(),
    }
}

/// Resamples to a square power-of-two tile: exact halvings in linear light when
/// possible, nearest-neighbour otherwise so pixel art stays crisp.
pub(in super::super) fn resample_square(texture: &DecodedTexture, tile: u32) -> Box<[u8]> {
    if texture.width == texture.height && texture.width.is_power_of_two() && texture.width >= tile {
        let mut pixels = texture.rgba8.clone();
        let mut size = texture.width;
        while size > tile {
            pixels = assets::downsample_linear_premultiplied(&pixels, size);
            size /= 2;
        }
        return pixels;
    }
    let mut pixels = Vec::with_capacity((tile * tile * 4) as usize);
    for y in 0..tile {
        let source_y = (u64::from(y) * u64::from(texture.height) / u64::from(tile)) as usize;
        for x in 0..tile {
            let source_x = (u64::from(x) * u64::from(texture.width) / u64::from(tile)) as usize;
            let offset = (source_y * texture.width as usize + source_x) * 4;
            pixels.extend_from_slice(&texture.rgba8[offset..offset + 4]);
        }
    }
    pixels.into_boxed_slice()
}
