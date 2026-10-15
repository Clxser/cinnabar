use crate::AlphaPolicy;

pub(crate) fn unpack(words: &[u32], bits: u8, out: &mut [u32]) {
    let per_word = 32 / usize::from(bits);
    let mask = (1_u32 << bits) - 1;
    for (word, slots) in words.iter().zip(out.chunks_mut(per_word)) {
        for (index, value) in slots.iter_mut().enumerate() {
            *value = (word >> (index * usize::from(bits))) & mask;
        }
    }
}

pub(crate) fn unpack_u16(words: &[u32], bits: u8, out: &mut [u16]) {
    let per_word = 32 / usize::from(bits);
    let mask = (1_u32 << bits) - 1;
    for (word, slots) in words.iter().zip(out.chunks_mut(per_word)) {
        for (index, value) in slots.iter_mut().enumerate() {
            *value = ((word >> (index * usize::from(bits))) & mask) as u16;
        }
    }
}

pub(crate) fn max_index(words: &[u32], bits: u8, count: usize) -> u32 {
    let per_word = 32 / usize::from(bits);
    let mask = (1_u32 << bits) - 1;
    (0..count)
        .map(|index| (words[index / per_word] >> ((index % per_word) * usize::from(bits))) & mask)
        .max()
        .unwrap_or(0)
}

pub(crate) fn sanitize(words: &mut [u32], bits: u8, palette_len: usize, count: usize) {
    let per_word = 32 / usize::from(bits);
    let mask = (1_u32 << bits) - 1;
    for index in 0..count {
        let shift = (index % per_word) * usize::from(bits);
        let word = &mut words[index / per_word];
        if ((*word >> shift) & mask) as usize >= palette_len {
            *word &= !(mask << shift);
        }
    }
}

pub(crate) fn columns(
    geometry: &[u64],
    occluders: &[u64],
    negative: bool,
    mask: u64,
    out: &mut [u64],
) {
    for ((&geometry, &occluders), out) in geometry.iter().zip(occluders).zip(out) {
        let adjacent = if negative {
            occluders << 1
        } else {
            occluders >> 1
        };
        *out = geometry & !adjacent & mask;
    }
}

pub(crate) fn light(low: &[[u8; 2]], high: &[[u8; 2]], out: &mut [[u8; 2]]) {
    for ((low, high), out) in low.iter().zip(high).zip(out) {
        *out = [low[0] | (high[0] << 4), low[1] | (high[1] << 4)];
    }
}

pub(crate) fn count_alpha(rgba: &[u8], threshold: u8) -> usize {
    rgba.chunks_exact(4)
        .filter(|pixel| pixel[3] >= threshold)
        .count()
}

pub(crate) fn mixed_alpha(rgba: &[u8], threshold: u8, covered: bool) -> bool {
    rgba.chunks_exact(4)
        .any(|pixel| (pixel[3] >= threshold) != covered)
}

pub(crate) fn alpha_matches(rgba: &[u8], policy: AlphaPolicy) -> bool {
    rgba.chunks_exact(4).all(|pixel| match policy {
        AlphaPolicy::Opaque => pixel[3] == 255,
        AlphaPolicy::Binary => matches!(pixel[3], 0 | 255),
    })
}

pub(crate) fn scale_alpha(rgba: &mut [u8], scale: u32, fraction_bits: u32) {
    let rounding = 1 << (fraction_bits - 1);
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[3] = ((u32::from(pixel[3]) * scale + rounding) >> fraction_bits).min(255) as u8;
    }
}
