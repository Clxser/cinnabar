//! Checked admission of immutable palette-native biome records from local recordings.

use std::collections::BTreeSet;

use super::*;
use crate::biome_lattice::{
    LATTICE_BIOME_LIMIT, LATTICE_POINT_WORDS, LATTICE_SIDE, LATTICE_STENCIL_SAMPLES,
    WIDE_LATTICE_POINT_WORDS,
};

impl PackedBiomeRecord {
    /// Checks an existing immutable record against a frozen tint table without copying its words.
    pub fn valid_for_recorded_tints(&self, tint_count: usize) -> bool {
        Self::from_recorded_words(Arc::clone(&self.words), tint_count).is_some()
    }

    /// Validates descriptors, palettes, packed indices and cached weights before any shader access.
    /// The caller supplies the exact frozen tint-table length referenced by this recording.
    pub fn from_recorded_words(words: Arc<[u32]>, tint_count: usize) -> Option<Self> {
        if tint_count == 0
            || !(DESCRIPTOR_WORDS + 2..=MAX_PACKED_BIOME_RECORD_WORDS).contains(&words.len())
            || !matches!(words[0], DESCRIPTOR_MAGIC | WIDE_DESCRIPTOR_MAGIC)
        {
            return None;
        }
        let uniform = (words[1] != NO_UNIFORM_TINT).then_some(words[1]);
        if uniform.is_some_and(|tint| tint as usize >= tint_count) {
            return None;
        }
        let point_words = if words[0] == DESCRIPTOR_MAGIC {
            LATTICE_POINT_WORDS
        } else {
            WIDE_LATTICE_POINT_WORDS
        };
        let payload_start = DESCRIPTOR_WORDS
            + if uniform.is_some() {
                0
            } else {
                LATTICE_SIDE.pow(3) * point_words
            };
        if payload_start >= words.len() || words[2 + CENTER_SLOT] == 0 {
            return None;
        }
        let record = Self { words };
        let mut intervals = BTreeSet::new();
        for &offset in &record.words[2..DESCRIPTOR_WORDS] {
            if offset == 0 {
                if uniform.is_some_and(|tint| tint != 0) {
                    return None;
                }
                continue;
            }
            let start = offset as usize;
            if start < payload_start {
                return None;
            }
            let header = *record.words.get(start)?;
            let bits = (header & BITS_MASK) as u8;
            let palette_len = ((header >> PALETTE_LEN_SHIFT) & PALETTE_LEN_MASK) as usize;
            if header & !(BITS_MASK | (PALETTE_LEN_MASK << PALETTE_LEN_SHIFT)) != 0
                || !world::supports_palette_bits(bits)
                || palette_len == 0
                || palette_len > BLOCKS_PER_SUB_CHUNK
                || bits == 0 && palette_len != 1
                || bits != 0 && palette_len > (1_usize << bits)
            {
                return None;
            }
            let payload = record.payload_at(offset)?;
            let palette_start = HEADER_WORDS + packed_word_count(usize::from(bits))?;
            let palette = &payload[palette_start..];
            if palette.iter().any(|&tint| {
                tint as usize >= tint_count || uniform.is_some_and(|value| value != tint)
            }) {
                return None;
            }
            if bits != 0 {
                let per_word = 32 / usize::from(bits);
                let mask = (1_u32 << bits) - 1;
                for index in 0..BLOCKS_PER_SUB_CHUNK {
                    let value = (payload[HEADER_WORDS + index / per_word]
                        >> ((index % per_word) * usize::from(bits)))
                        & mask;
                    if value as usize >= palette_len {
                        return None;
                    }
                }
            }
            intervals.insert((start, start.checked_add(payload.len())?));
        }
        let mut end = payload_start;
        for (start, next) in intervals {
            if start != end {
                return None;
            }
            end = next;
        }
        if end != record.words.len() {
            return None;
        }
        if uniform.is_none()
            && !valid_lattice(
                &record.words[DESCRIPTOR_WORDS..payload_start],
                point_words,
                tint_count,
            )
        {
            return None;
        }
        Some(record)
    }
}

/// Cached interpolation data must address this palette and carry finite nonnegative weights.
fn valid_lattice(words: &[u32], stride: usize, tint_count: usize) -> bool {
    for point in words.chunks_exact(stride) {
        let count = point[0] as usize;
        if !(1..=LATTICE_BIOME_LIMIT).contains(&count) {
            return false;
        }
        let mut weight_sum = 0.0_f32;
        for index in 0..LATTICE_BIOME_LIMIT {
            let (tint, weight) = if stride == LATTICE_POINT_WORDS {
                let tint = (point[1 + index / 2] >> ((index % 2) * u16::BITS as usize))
                    & u32::from(u16::MAX);
                let weight = ((point[stride - 1] >> (index * u8::BITS as usize))
                    & u32::from(u8::MAX)) as f32
                    / LATTICE_STENCIL_SAMPLES as f32;
                (tint, weight)
            } else {
                (
                    point[1 + index],
                    f32::from_bits(point[1 + LATTICE_BIOME_LIMIT + index]),
                )
            };
            if tint as usize >= tint_count
                || !weight.is_finite()
                || !(0.0..=1.0).contains(&weight)
                || index < count && weight == 0.0
                || index >= count && weight != 0.0
            {
                return false;
            }
            weight_sum += weight;
        }
        if weight_sum > 1.0 + f32::EPSILON * LATTICE_BIOME_LIMIT as f32 {
            return false;
        }
    }
    true
}
