use super::*;

#[target_feature(enable = "avx2")]
pub(crate) unsafe fn unpack_avx2(words: &[u32], bits: u8, out: &mut [u32]) {
    // Byte/halfword expansion needs no per-index shift or lane permutation.
    if matches!(bits, 8 | 16) {
        let mut index = 0;
        // SAFETY: eight outputs require exactly eight or sixteen source bytes.
        unsafe {
            while index + 8 <= out.len() {
                let source = words
                    .as_ptr()
                    .cast::<u8>()
                    .add(index * usize::from(bits) / 8);
                let expanded = if bits == 8 {
                    _mm256_cvtepu8_epi32(_mm_loadl_epi64(source.cast()))
                } else {
                    _mm256_cvtepu16_epi32(_mm_loadu_si128(source.cast()))
                };
                _mm256_storeu_si256(out.as_mut_ptr().add(index).cast(), expanded);
                index += 8;
            }
        }
        let per_word = 32 / usize::from(bits);
        scalar::unpack(&words[index / per_word..], bits, &mut out[index..]);
        return;
    }
    let per_word = 32 / usize::from(bits);
    let mask = _mm256_set1_epi32(((1_u32 << bits) - 1) as i32);
    let lanes = _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7);
    let shifts = _mm256_mullo_epi32(lanes, _mm256_set1_epi32(i32::from(bits)));
    for (&word, slots) in words.iter().zip(out.chunks_mut(per_word)) {
        let input = _mm256_set1_epi32(word as i32);
        for (base, batch) in slots.chunks_mut(8).enumerate() {
            let shift = _mm256_add_epi32(
                shifts,
                _mm256_set1_epi32((base * 8 * usize::from(bits)) as i32),
            );
            let indices = _mm256_and_si256(_mm256_srlv_epi32(input, shift), mask);
            // SAFETY: full stores have eight destinations; masked stores touch only live lanes.
            unsafe {
                if batch.len() == 8 {
                    _mm256_storeu_si256(batch.as_mut_ptr().cast(), indices);
                } else {
                    let valid = _mm256_cmpgt_epi32(_mm256_set1_epi32(batch.len() as i32), lanes);
                    _mm256_maskstore_epi32(batch.as_mut_ptr().cast(), valid, indices);
                }
            }
        }
    }
}

#[target_feature(enable = "sse2")]
pub(crate) unsafe fn unpack_sse2(words: &[u32], bits: u8, out: &mut [u32]) {
    let mut index = 0;
    let zero = _mm_setzero_si128();
    // SAFETY: complete pixel-width groups occupy exactly sixteen source bytes.
    unsafe {
        if bits == 8 {
            while index + 16 <= out.len() {
                let source = _mm_loadu_si128(words.as_ptr().cast::<u8>().add(index).cast());
                let low = _mm_unpacklo_epi8(source, zero);
                let high = _mm_unpackhi_epi8(source, zero);
                _mm_storeu_si128(
                    out.as_mut_ptr().add(index).cast(),
                    _mm_unpacklo_epi16(low, zero),
                );
                _mm_storeu_si128(
                    out.as_mut_ptr().add(index + 4).cast(),
                    _mm_unpackhi_epi16(low, zero),
                );
                _mm_storeu_si128(
                    out.as_mut_ptr().add(index + 8).cast(),
                    _mm_unpacklo_epi16(high, zero),
                );
                _mm_storeu_si128(
                    out.as_mut_ptr().add(index + 12).cast(),
                    _mm_unpackhi_epi16(high, zero),
                );
                index += 16;
            }
        } else if bits == 16 {
            while index + 8 <= out.len() {
                let source = _mm_loadu_si128(words.as_ptr().cast::<u8>().add(index * 2).cast());
                _mm_storeu_si128(
                    out.as_mut_ptr().add(index).cast(),
                    _mm_unpacklo_epi16(source, zero),
                );
                _mm_storeu_si128(
                    out.as_mut_ptr().add(index + 4).cast(),
                    _mm_unpackhi_epi16(source, zero),
                );
                index += 8;
            }
        }
    }
    scalar::unpack(
        &words[index / (32 / usize::from(bits))..],
        bits,
        &mut out[index..],
    );
}

#[target_feature(enable = "avx2")]
pub(crate) unsafe fn max_avx2(words: &[u32], bits: u8, count: usize) -> u32 {
    let per_word = 32 / usize::from(bits);
    let full_words = count / per_word;
    let mut index = 0;
    let mut maximum = _mm256_setzero_si256();
    let mask = _mm256_set1_epi32(((1_u32 << bits) - 1) as i32);
    // SAFETY: complete source words only; every extracted value is within 16 bits.
    unsafe {
        while index + 8 <= full_words {
            let input = _mm256_loadu_si256(words.as_ptr().add(index).cast());
            for slot in 0..per_word {
                let shift = _mm_cvtsi32_si128((slot * usize::from(bits)) as i32);
                let values = _mm256_and_si256(_mm256_srl_epi32(input, shift), mask);
                maximum = _mm256_max_epu32(maximum, values);
            }
            index += 8;
        }
        let mut lanes = [0_u32; 8];
        _mm256_storeu_si256(lanes.as_mut_ptr().cast(), maximum);
        lanes.into_iter().max().unwrap_or(0).max(scalar::max_index(
            &words[index..],
            bits,
            count - index * per_word,
        ))
    }
}

#[target_feature(enable = "sse2")]
pub(crate) unsafe fn max_sse2(words: &[u32], bits: u8, count: usize) -> u32 {
    let per_word = 32 / usize::from(bits);
    let full_words = count / per_word;
    let mut index = 0;
    let mut maximum = _mm_setzero_si128();
    let mask = _mm_set1_epi32(((1_u32 << bits) - 1) as i32);
    // SAFETY: complete source words only; masked values are nonnegative signed i32s.
    unsafe {
        while index + 4 <= full_words {
            let input = _mm_loadu_si128(words.as_ptr().add(index).cast());
            for slot in 0..per_word {
                let shift = _mm_cvtsi32_si128((slot * usize::from(bits)) as i32);
                let values = _mm_and_si128(_mm_srl_epi32(input, shift), mask);
                let greater = _mm_cmpgt_epi32(values, maximum);
                maximum = _mm_or_si128(
                    _mm_and_si128(greater, values),
                    _mm_andnot_si128(greater, maximum),
                );
            }
            index += 4;
        }
        let mut lanes = [0_u32; 4];
        _mm_storeu_si128(lanes.as_mut_ptr().cast(), maximum);
        lanes.into_iter().max().unwrap_or(0).max(scalar::max_index(
            &words[index..],
            bits,
            count - index * per_word,
        ))
    }
}

#[target_feature(enable = "avx2")]
pub(crate) unsafe fn sanitize_avx2(words: &mut [u32], bits: u8, palette_len: usize, count: usize) {
    let per_word = 32 / usize::from(bits);
    let full_words = count / per_word;
    let mut index = 0;
    let raw_mask = (1_u32 << bits) - 1;
    let mask = _mm256_set1_epi32(raw_mask as i32);
    let maximum = _mm256_set1_epi32(palette_len as i32 - 1);
    // SAFETY: complete groups of eight words; padding bits never enter a clear mask.
    unsafe {
        while index + 8 <= full_words {
            let input = _mm256_loadu_si256(words.as_ptr().add(index).cast());
            let mut clear = _mm256_setzero_si256();
            for slot in 0..per_word {
                let shift = slot * usize::from(bits);
                let values = _mm256_and_si256(
                    _mm256_srl_epi32(input, _mm_cvtsi32_si128(shift as i32)),
                    mask,
                );
                let invalid = _mm256_cmpgt_epi32(values, maximum);
                clear = _mm256_or_si256(
                    clear,
                    _mm256_and_si256(invalid, _mm256_set1_epi32((raw_mask << shift) as i32)),
                );
            }
            _mm256_storeu_si256(
                words.as_mut_ptr().add(index).cast(),
                _mm256_andnot_si256(clear, input),
            );
            index += 8;
        }
    }
    scalar::sanitize(
        &mut words[index..],
        bits,
        palette_len,
        count - index * per_word,
    );
}

#[target_feature(enable = "sse2")]
pub(crate) unsafe fn sanitize_sse2(words: &mut [u32], bits: u8, palette_len: usize, count: usize) {
    let per_word = 32 / usize::from(bits);
    let full_words = count / per_word;
    let mut index = 0;
    let raw_mask = (1_u32 << bits) - 1;
    let mask = _mm_set1_epi32(raw_mask as i32);
    let maximum = _mm_set1_epi32(palette_len as i32 - 1);
    // SAFETY: complete groups of four words; padding bits never enter a clear mask.
    unsafe {
        while index + 4 <= full_words {
            let input = _mm_loadu_si128(words.as_ptr().add(index).cast());
            let mut clear = _mm_setzero_si128();
            for slot in 0..per_word {
                let shift = slot * usize::from(bits);
                let values =
                    _mm_and_si128(_mm_srl_epi32(input, _mm_cvtsi32_si128(shift as i32)), mask);
                let invalid = _mm_cmpgt_epi32(values, maximum);
                clear = _mm_or_si128(
                    clear,
                    _mm_and_si128(invalid, _mm_set1_epi32((raw_mask << shift) as i32)),
                );
            }
            _mm_storeu_si128(
                words.as_mut_ptr().add(index).cast(),
                _mm_andnot_si128(clear, input),
            );
            index += 4;
        }
    }
    scalar::sanitize(
        &mut words[index..],
        bits,
        palette_len,
        count - index * per_word,
    );
}
