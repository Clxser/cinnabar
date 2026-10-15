use super::{backends, words};
use crate::{
    Backend, max_packed_index, sanitize_packed_indices, scalar, unpack_indices, unpack_indices_u16,
};

fn unpack(backend: Backend, words: &[u32], bits: u8, out: &mut [u32]) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: backends() checked instruction support; test slices contain every requested word.
    unsafe {
        match backend {
            Backend::Avx2 => return crate::x86::unpack_avx2(words, bits, out),
            Backend::Sse2 => return crate::x86::unpack_sse2(words, bits, out),
            Backend::Scalar => (),
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    let _ = backend;
    scalar::unpack(words, bits, out);
}

fn maximum(backend: Backend, words: &[u32], bits: u8, count: usize) -> u32 {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: backends() checked instruction support; count fits the packed source.
    unsafe {
        match backend {
            Backend::Avx2 => return crate::x86::max_avx2(words, bits, count),
            Backend::Sse2 => return crate::x86::max_sse2(words, bits, count),
            Backend::Scalar => (),
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    let _ = backend;
    scalar::max_index(words, bits, count)
}

fn unpack_u16(backend: Backend, words: &[u32], bits: u8, out: &mut [u16]) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: backends() checked support; every output index has a source word.
    unsafe {
        match backend {
            Backend::Avx2 => return crate::x86::unpack_u16_avx2(words, bits, out),
            Backend::Sse2 => return crate::x86::unpack_u16_sse2(words, bits, out),
            Backend::Scalar => (),
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    let _ = backend;
    scalar::unpack_u16(words, bits, out);
}

fn sanitize(backend: Backend, words: &mut [u32], bits: u8, length: usize, count: usize) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: backends() checked instruction support; counts and palette bounds are valid.
    unsafe {
        match backend {
            Backend::Avx2 => return crate::x86::sanitize_avx2(words, bits, length, count),
            Backend::Sse2 => return crate::x86::sanitize_sse2(words, bits, length, count),
            Backend::Scalar => (),
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    let _ = backend;
    scalar::sanitize(words, bits, length, count);
}

#[test]
fn unpack_and_maximum_match_independent_word_reference_with_unaligned_tails() {
    for bits in 1..=16 {
        let per_word = 32 / usize::from(bits);
        let mask = (1_u32 << bits) - 1;
        for count in [0_usize, 1, 3, 7, 8, 9, 15, 16, 17, 31, 32, 33, 4096] {
            let source = words(count.div_ceil(per_word) + 2);
            let input = &source[1..source.len() - 1];
            let expected: Vec<_> = (0..count)
                .map(|index| {
                    let shift = (index % per_word) * usize::from(bits);
                    ((u64::from(input[index / per_word]) >> shift) & u64::from(mask)) as u32
                })
                .collect();
            for backend in backends() {
                let mut out = vec![0xdead_beef; count + 2];
                unpack(backend, input, bits, &mut out[1..count + 1]);
                assert_eq!(
                    &out[1..count + 1],
                    expected,
                    "{backend:?}, width {bits}, count {count}"
                );
                assert_eq!(out[0], 0xdead_beef);
                assert_eq!(out[count + 1], 0xdead_beef);
                let mut narrow = vec![0xbeef; count + 2];
                unpack_u16(backend, input, bits, &mut narrow[1..count + 1]);
                assert!(
                    narrow[1..count + 1]
                        .iter()
                        .zip(&expected)
                        .all(|(&a, &b)| u32::from(a) == b)
                );
                assert_eq!(narrow[0], 0xbeef);
                assert_eq!(narrow[count + 1], 0xbeef);
                assert_eq!(
                    maximum(backend, input, bits, count),
                    expected.iter().copied().max().unwrap_or(0)
                );
            }
            let mut dispatched = vec![0; count];
            unpack_indices(input, bits, &mut dispatched);
            assert_eq!(dispatched, expected);
            let mut narrow = vec![0; count];
            unpack_indices_u16(input, bits, &mut narrow);
            assert!(
                narrow
                    .iter()
                    .zip(&expected)
                    .all(|(&a, &b)| u32::from(a) == b)
            );
            assert_eq!(
                max_packed_index(input, bits, count),
                expected.iter().copied().max().or(Some(0))
            );
        }
    }
}

#[test]
fn sanitizing_preserves_padding_and_unprocessed_indices() {
    for bits in 1..=16 {
        let per_word = 32 / usize::from(bits);
        let capacity = 1_usize << bits;
        for count in [0_usize, 1, 7, 17, 31, 33, 4096] {
            let original = words(count.div_ceil(per_word) + 3);
            for length in [0, 1, capacity / 2, capacity - 1] {
                let mut expected = original.clone();
                scalar::sanitize(&mut expected[1..], bits, length, count);
                for backend in backends() {
                    let mut actual = original.clone();
                    sanitize(backend, &mut actual[1..], bits, length, count);
                    assert_eq!(
                        actual, expected,
                        "{backend:?}, width {bits}, palette {length}"
                    );
                }
                let mut actual = original.clone();
                sanitize_packed_indices(&mut actual[1..], bits, length, count);
                assert_eq!(actual, expected);
            }
        }
    }
}

#[test]
fn public_boundaries_handle_missing_words_and_padding() {
    let mut out = [99; 13];
    unpack_indices(&[0xffff_ffff], 3, &mut out);
    assert_eq!(&out[..10], &[7; 10]);
    assert_eq!(&out[10..], &[u32::MAX; 3]);
    unpack_indices(&[], 0, &mut out);
    assert_eq!(out, [0; 13]);
    unpack_indices(&[], 32, &mut out);
    assert_eq!(out, [u32::MAX; 13]);
    let mut narrow = [99; 13];
    unpack_indices_u16(&[u32::MAX], 3, &mut narrow);
    assert_eq!(&narrow[..10], &[7; 10]);
    assert_eq!(&narrow[10..], &[u16::MAX; 3]);
    unpack_indices_u16(&[], 0, &mut narrow);
    assert_eq!(narrow, [0; 13]);
    unpack_indices_u16(&[], 32, &mut narrow);
    assert_eq!(narrow, [u16::MAX; 13]);
    assert_eq!(max_packed_index(&[0xffff_ffff], 3, 11), None);
    assert_eq!(max_packed_index(&[0xffff_fff8], 3, 1), Some(0));
    assert_eq!(max_packed_index(&[], 17, 1), None);
    let mut padded = [0xffff_ffff];
    sanitize_packed_indices(&mut padded, 3, 1, 13);
    assert_eq!(padded, [0xc000_0000]);
    sanitize_packed_indices(&mut padded, 0, 0, 4096);
    assert_eq!(padded, [0xc000_0000]);
}
