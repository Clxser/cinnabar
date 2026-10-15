use super::*;

#[target_feature(enable = "avx2")]
pub(crate) unsafe fn unpack_u16_avx2(words: &[u32], bits: u8, out: &mut [u16]) {
    if matches!(bits, 8 | 16) {
        let mut index = 0;
        // SAFETY: sixteen outputs need exactly sixteen or thirty-two source bytes.
        unsafe {
            while index + 16 <= out.len() {
                let source = words
                    .as_ptr()
                    .cast::<u8>()
                    .add(index * usize::from(bits) / 8);
                let indices = if bits == 8 {
                    _mm256_cvtepu8_epi16(_mm_loadu_si128(source.cast()))
                } else {
                    _mm256_loadu_si256(source.cast())
                };
                _mm256_storeu_si256(out.as_mut_ptr().add(index).cast(), indices);
                index += 16;
            }
        }
        scalar::unpack_u16(
            &words[index / (32 / usize::from(bits))..],
            bits,
            &mut out[index..],
        );
        return;
    }
    let per_word = 32 / usize::from(bits);
    let lanes = _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7);
    let shifts = _mm256_mullo_epi32(lanes, _mm256_set1_epi32(i32::from(bits)));
    let mask = _mm256_set1_epi32(((1_u32 << bits) - 1) as i32);
    let zero = _mm256_setzero_si256();
    for (&word, slots) in words.iter().zip(out.chunks_mut(per_word)) {
        let input = _mm256_set1_epi32(word as i32);
        for (base, batch) in slots.chunks_mut(8).enumerate() {
            let shift = _mm256_add_epi32(
                shifts,
                _mm256_set1_epi32((base * 8 * usize::from(bits)) as i32),
            );
            let indices = _mm256_and_si256(_mm256_srlv_epi32(input, shift), mask);
            let pairs = _mm256_packus_epi32(indices, zero);
            let compact = _mm_unpacklo_epi64(
                _mm256_castsi256_si128(pairs),
                _mm256_extracti128_si256::<1>(pairs),
            );
            // SAFETY: full stores touch eight destinations. Partial batches store into a local
            // complete vector and copy only live lanes into the output slice.
            unsafe {
                if batch.len() == 8 {
                    _mm_storeu_si128(batch.as_mut_ptr().cast(), compact);
                } else {
                    let mut tail = [0_u16; 8];
                    _mm_storeu_si128(tail.as_mut_ptr().cast(), compact);
                    batch.copy_from_slice(&tail[..batch.len()]);
                }
            }
        }
    }
}

#[target_feature(enable = "sse2")]
pub(crate) unsafe fn unpack_u16_sse2(words: &[u32], bits: u8, out: &mut [u16]) {
    let mut index = 0;
    let zero = _mm_setzero_si128();
    // SAFETY: batches consume sixteen source bytes, with bounded unaligned output stores.
    unsafe {
        if bits == 8 {
            while index + 16 <= out.len() {
                let source = _mm_loadu_si128(words.as_ptr().cast::<u8>().add(index).cast());
                _mm_storeu_si128(
                    out.as_mut_ptr().add(index).cast(),
                    _mm_unpacklo_epi8(source, zero),
                );
                _mm_storeu_si128(
                    out.as_mut_ptr().add(index + 8).cast(),
                    _mm_unpackhi_epi8(source, zero),
                );
                index += 16;
            }
        } else if bits == 16 {
            while index + 8 <= out.len() {
                let source = _mm_loadu_si128(words.as_ptr().cast::<u8>().add(index * 2).cast());
                _mm_storeu_si128(out.as_mut_ptr().add(index).cast(), source);
                index += 8;
            }
        }
    }
    scalar::unpack_u16(
        &words[index / (32 / usize::from(bits))..],
        bits,
        &mut out[index..],
    );
}
