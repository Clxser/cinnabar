use super::*;
use crate::AlphaPolicy;

#[target_feature(enable = "avx2")]
pub(crate) unsafe fn mixed_alpha_avx2(rgba: &[u8], threshold: u8, covered: bool) -> bool {
    let mut index = 0;
    let vector_threshold = _mm256_set1_epi32(i32::from(threshold) - 1);
    let expected = if covered { -1 } else { 0 };
    // SAFETY: every iteration reads eight complete pixels, without accessing the tail.
    unsafe {
        while index + 32 <= rgba.len() {
            let alpha =
                _mm256_srli_epi32::<24>(_mm256_loadu_si256(rgba.as_ptr().add(index).cast()));
            if _mm256_movemask_epi8(_mm256_cmpgt_epi32(alpha, vector_threshold)) != expected {
                return true;
            }
            index += 32;
        }
    }
    scalar::mixed_alpha(&rgba[index..], threshold, covered)
}

#[target_feature(enable = "sse2")]
pub(crate) unsafe fn mixed_alpha_sse2(rgba: &[u8], threshold: u8, covered: bool) -> bool {
    let mut index = 0;
    let vector_threshold = _mm_set1_epi32(i32::from(threshold) - 1);
    let expected = if covered { 0xffff } else { 0 };
    // SAFETY: every iteration reads four complete pixels, without accessing the tail.
    unsafe {
        while index + 16 <= rgba.len() {
            let alpha = _mm_srli_epi32::<24>(_mm_loadu_si128(rgba.as_ptr().add(index).cast()));
            if _mm_movemask_epi8(_mm_cmpgt_epi32(alpha, vector_threshold)) != expected {
                return true;
            }
            index += 16;
        }
    }
    scalar::mixed_alpha(&rgba[index..], threshold, covered)
}

#[target_feature(enable = "avx2")]
pub(crate) unsafe fn count_alpha_avx2(rgba: &[u8], threshold: u8) -> usize {
    let mut index = 0;
    let mut count = 0;
    let vector_threshold = _mm256_set1_epi32(i32::from(threshold) - 1);
    // SAFETY: each vector reads eight complete RGBA8 pixels in little-endian x86 byte order.
    unsafe {
        while index + 32 <= rgba.len() {
            let pixels = _mm256_loadu_si256(rgba.as_ptr().add(index).cast());
            let alpha = _mm256_srli_epi32::<24>(pixels);
            let accepted = _mm256_cmpgt_epi32(alpha, vector_threshold);
            count +=
                (_mm256_movemask_ps(_mm256_castsi256_ps(accepted)) as u32).count_ones() as usize;
            index += 32;
        }
    }
    count + scalar::count_alpha(&rgba[index..], threshold)
}

#[target_feature(enable = "sse2")]
pub(crate) unsafe fn count_alpha_sse2(rgba: &[u8], threshold: u8) -> usize {
    let mut index = 0;
    let mut count = 0;
    let vector_threshold = _mm_set1_epi32(i32::from(threshold) - 1);
    // SAFETY: each vector reads four complete RGBA8 pixels.
    unsafe {
        while index + 16 <= rgba.len() {
            let pixels = _mm_loadu_si128(rgba.as_ptr().add(index).cast());
            let alpha = _mm_srli_epi32::<24>(pixels);
            let accepted = _mm_cmpgt_epi32(alpha, vector_threshold);
            count += (_mm_movemask_ps(_mm_castsi128_ps(accepted)) as u32).count_ones() as usize;
            index += 16;
        }
    }
    count + scalar::count_alpha(&rgba[index..], threshold)
}

#[target_feature(enable = "avx2")]
pub(crate) unsafe fn alpha_matches_avx2(rgba: &[u8], policy: AlphaPolicy) -> bool {
    let mut index = 0;
    let zero = _mm256_setzero_si256();
    let opaque = _mm256_set1_epi32(255);
    // SAFETY: each vector reads eight complete RGBA8 pixels.
    unsafe {
        while index + 32 <= rgba.len() {
            let alpha =
                _mm256_srli_epi32::<24>(_mm256_loadu_si256(rgba.as_ptr().add(index).cast()));
            let mut accepted = _mm256_cmpeq_epi32(alpha, opaque);
            if matches!(policy, AlphaPolicy::Binary) {
                accepted = _mm256_or_si256(accepted, _mm256_cmpeq_epi32(alpha, zero));
            }
            if _mm256_movemask_epi8(accepted) != -1 {
                return false;
            }
            index += 32;
        }
    }
    scalar::alpha_matches(&rgba[index..], policy)
}

#[target_feature(enable = "sse2")]
pub(crate) unsafe fn alpha_matches_sse2(rgba: &[u8], policy: AlphaPolicy) -> bool {
    let mut index = 0;
    let zero = _mm_setzero_si128();
    let opaque = _mm_set1_epi32(255);
    // SAFETY: each vector reads four complete RGBA8 pixels.
    unsafe {
        while index + 16 <= rgba.len() {
            let alpha = _mm_srli_epi32::<24>(_mm_loadu_si128(rgba.as_ptr().add(index).cast()));
            let mut accepted = _mm_cmpeq_epi32(alpha, opaque);
            if matches!(policy, AlphaPolicy::Binary) {
                accepted = _mm_or_si128(accepted, _mm_cmpeq_epi32(alpha, zero));
            }
            if _mm_movemask_epi8(accepted) != 0xffff {
                return false;
            }
            index += 16;
        }
    }
    scalar::alpha_matches(&rgba[index..], policy)
}

#[target_feature(enable = "avx2")]
pub(crate) unsafe fn scale_alpha_avx2(rgba: &mut [u8], scale: u32, fraction_bits: u32) {
    let mut index = 0;
    let multiplier = _mm256_set1_epi32(scale as i32);
    let rounding = _mm256_set1_epi32((1_u32 << (fraction_bits - 1)) as i32);
    let shift = _mm_cvtsi32_si128(fraction_bits as i32);
    let alpha_mask = _mm256_set1_epi32(255);
    let rgb_mask = _mm256_set1_epi32(0x00ff_ffff);
    // SAFETY: validated arithmetic fits u32; stores modify only complete pixels.
    unsafe {
        while index + 32 <= rgba.len() {
            let pixels = _mm256_loadu_si256(rgba.as_ptr().add(index).cast());
            let alpha = _mm256_srli_epi32::<24>(pixels);
            let scaled = _mm256_srl_epi32(
                _mm256_add_epi32(_mm256_mullo_epi32(alpha, multiplier), rounding),
                shift,
            );
            let clamped = _mm256_min_epu32(scaled, alpha_mask);
            let result = _mm256_or_si256(
                _mm256_and_si256(pixels, rgb_mask),
                _mm256_slli_epi32::<24>(clamped),
            );
            _mm256_storeu_si256(rgba.as_mut_ptr().add(index).cast(), result);
            index += 32;
        }
    }
    scalar::scale_alpha(&mut rgba[index..], scale, fraction_bits);
}

#[target_feature(enable = "sse2")]
pub(crate) unsafe fn scale_alpha_sse2(rgba: &mut [u8], scale: u32, fraction_bits: u32) {
    let mut index = 0;
    let multiplier = _mm_set1_epi32(scale as i32);
    let rounding = _mm_set1_epi32((1_u32 << (fraction_bits - 1)) as i32);
    let shift = _mm_cvtsi32_si128(fraction_bits as i32);
    let opaque = _mm_set1_epi32(255);
    let rgb_mask = _mm_set1_epi32(0x00ff_ffff);
    // SAFETY: validated products fit u32; each iteration reads and writes four complete pixels.
    unsafe {
        while index + 16 <= rgba.len() {
            let pixels = _mm_loadu_si128(rgba.as_ptr().add(index).cast());
            let alpha = _mm_srli_epi32::<24>(pixels);
            let even = _mm_mul_epu32(alpha, multiplier);
            let odd = _mm_mul_epu32(_mm_srli_si128::<4>(alpha), multiplier);
            let products = _mm_unpacklo_epi32(
                _mm_shuffle_epi32::<0x88>(even),
                _mm_shuffle_epi32::<0x88>(odd),
            );
            let scaled = _mm_srl_epi32(_mm_add_epi32(products, rounding), shift);
            // The validated shift is at least one, so these unsigned results fit signed i32.
            let excess = _mm_cmpgt_epi32(scaled, opaque);
            let clamped = _mm_or_si128(
                _mm_and_si128(excess, opaque),
                _mm_andnot_si128(excess, scaled),
            );
            let result = _mm_or_si128(
                _mm_and_si128(pixels, rgb_mask),
                _mm_slli_epi32::<24>(clamped),
            );
            _mm_storeu_si128(rgba.as_mut_ptr().add(index).cast(), result);
            index += 16;
        }
    }
    scalar::scale_alpha(&mut rgba[index..], scale, fraction_bits);
}
