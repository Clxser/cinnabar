#[cfg(target_arch = "x86")]
use std::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

mod packed;
mod texture;
pub(super) use packed::{
    max_avx2, max_sse2, sanitize_avx2, sanitize_sse2, unpack_avx2, unpack_sse2,
};
pub(super) use packed_u16::{unpack_u16_avx2, unpack_u16_sse2};
pub(super) use texture::{
    alpha_matches_avx2, alpha_matches_sse2, count_alpha_avx2, count_alpha_sse2, scale_alpha_avx2,
    scale_alpha_sse2,
};
pub(super) use texture::{mixed_alpha_avx2, mixed_alpha_sse2};

use crate::scalar;
mod packed_u16;

#[target_feature(enable = "avx2")]
pub(super) unsafe fn equal_avx2(values: &[u8], value: u8) -> bool {
    let mut index = 0;
    let expected = _mm256_set1_epi8(value as i8);
    // SAFETY: complete 32-byte blocks only; callers checked AVX2 support.
    unsafe {
        while index + 32 <= values.len() {
            let input = _mm256_loadu_si256(values.as_ptr().add(index).cast());
            if _mm256_movemask_epi8(_mm256_cmpeq_epi8(input, expected)) != -1 {
                return false;
            }
            index += 32;
        }
    }
    values[index..].iter().all(|&entry| entry == value)
}

#[target_feature(enable = "sse2")]
pub(super) unsafe fn equal_sse2(values: &[u8], value: u8) -> bool {
    let mut index = 0;
    let expected = _mm_set1_epi8(value as i8);
    // SAFETY: complete 16-byte blocks only; callers checked SSE2 support.
    unsafe {
        while index + 16 <= values.len() {
            let input = _mm_loadu_si128(values.as_ptr().add(index).cast());
            if _mm_movemask_epi8(_mm_cmpeq_epi8(input, expected)) != 0xffff {
                return false;
            }
            index += 16;
        }
    }
    values[index..].iter().all(|&entry| entry == value)
}

#[target_feature(enable = "avx2")]
pub(super) unsafe fn columns_avx2(
    geometry: &[u64],
    occluders: &[u64],
    negative: bool,
    mask: u64,
    out: &mut [u64],
) {
    let mut index = 0;
    let vector_mask = _mm256_set1_epi64x(mask as i64);
    // SAFETY: callers validated equal slice lengths; each iteration touches four u64s.
    unsafe {
        while index + 4 <= out.len() {
            let geometry = _mm256_loadu_si256(geometry.as_ptr().add(index).cast());
            let blockers = _mm256_loadu_si256(occluders.as_ptr().add(index).cast());
            let adjacent = if negative {
                _mm256_slli_epi64::<1>(blockers)
            } else {
                _mm256_srli_epi64::<1>(blockers)
            };
            let faces = _mm256_and_si256(_mm256_andnot_si256(adjacent, geometry), vector_mask);
            _mm256_storeu_si256(out.as_mut_ptr().add(index).cast(), faces);
            index += 4;
        }
    }
    scalar::columns(
        &geometry[index..],
        &occluders[index..],
        negative,
        mask,
        &mut out[index..],
    );
}

#[target_feature(enable = "sse2")]
pub(super) unsafe fn columns_sse2(
    geometry: &[u64],
    occluders: &[u64],
    negative: bool,
    mask: u64,
    out: &mut [u64],
) {
    let mut index = 0;
    let vector_mask = _mm_set1_epi64x(mask as i64);
    // SAFETY: callers validated equal slice lengths; each iteration touches two u64s.
    unsafe {
        while index + 2 <= out.len() {
            let geometry = _mm_loadu_si128(geometry.as_ptr().add(index).cast());
            let blockers = _mm_loadu_si128(occluders.as_ptr().add(index).cast());
            let adjacent = if negative {
                _mm_slli_epi64::<1>(blockers)
            } else {
                _mm_srli_epi64::<1>(blockers)
            };
            let faces = _mm_and_si128(_mm_andnot_si128(adjacent, geometry), vector_mask);
            _mm_storeu_si128(out.as_mut_ptr().add(index).cast(), faces);
            index += 2;
        }
    }
    scalar::columns(
        &geometry[index..],
        &occluders[index..],
        negative,
        mask,
        &mut out[index..],
    );
}

#[target_feature(enable = "avx2")]
pub(super) unsafe fn light_avx2(low: &[[u8; 2]], high: &[[u8; 2]], out: &mut [[u8; 2]]) {
    let mut index = 0;
    let high_mask = _mm256_set1_epi8(0xf0_u8 as i8);
    // SAFETY: complete groups of sixteen two-byte pairs stay inside all validated slices.
    unsafe {
        while index + 16 <= out.len() {
            let a = _mm256_loadu_si256(low.as_ptr().add(index).cast());
            let b = _mm256_loadu_si256(high.as_ptr().add(index).cast());
            let high_nibbles = _mm256_and_si256(_mm256_slli_epi16::<4>(b), high_mask);
            _mm256_storeu_si256(
                out.as_mut_ptr().add(index).cast(),
                _mm256_or_si256(a, high_nibbles),
            );
            index += 16;
        }
    }
    scalar::light(&low[index..], &high[index..], &mut out[index..]);
}

#[target_feature(enable = "sse2")]
pub(super) unsafe fn light_sse2(low: &[[u8; 2]], high: &[[u8; 2]], out: &mut [[u8; 2]]) {
    let mut index = 0;
    let high_mask = _mm_set1_epi8(0xf0_u8 as i8);
    // SAFETY: complete groups of eight two-byte pairs stay inside all validated slices.
    unsafe {
        while index + 8 <= out.len() {
            let a = _mm_loadu_si128(low.as_ptr().add(index).cast());
            let b = _mm_loadu_si128(high.as_ptr().add(index).cast());
            let high_nibbles = _mm_and_si128(_mm_slli_epi16::<4>(b), high_mask);
            _mm_storeu_si128(
                out.as_mut_ptr().add(index).cast(),
                _mm_or_si128(a, high_nibbles),
            );
            index += 8;
        }
    }
    scalar::light(&low[index..], &high[index..], &mut out[index..]);
}
