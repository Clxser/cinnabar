use super::{backends, words};
use crate::{Backend, all_equal_u8, exposed_columns, pack_light_pairs, scalar};

fn columns(
    backend: Backend,
    geometry: &[u64],
    occluders: &[u64],
    negative: bool,
    mask: u64,
    out: &mut [u64],
) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: backends() checked CPU support; test slices have equal lengths.
    unsafe {
        match backend {
            Backend::Avx2 => {
                return crate::x86::columns_avx2(geometry, occluders, negative, mask, out);
            }
            Backend::Sse2 => {
                return crate::x86::columns_sse2(geometry, occluders, negative, mask, out);
            }
            Backend::Scalar => (),
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    let _ = backend;
    scalar::columns(geometry, occluders, negative, mask, out);
}

fn light(backend: Backend, low: &[[u8; 2]], high: &[[u8; 2]], out: &mut [[u8; 2]]) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: backends() checked CPU support; test slices have equal lengths.
    unsafe {
        match backend {
            Backend::Avx2 => return crate::x86::light_avx2(low, high, out),
            Backend::Sse2 => return crate::x86::light_sse2(low, high, out),
            Backend::Scalar => (),
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    let _ = backend;
    scalar::light(low, high, out);
}

fn equal(backend: Backend, values: &[u8], value: u8) -> bool {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: backends() checked CPU support; kernels bound every read by the slice.
    unsafe {
        match backend {
            Backend::Avx2 => return crate::x86::equal_avx2(values, value),
            Backend::Sse2 => return crate::x86::equal_sse2(values, value),
            Backend::Scalar => (),
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    let _ = backend;
    values.iter().all(|&entry| entry == value)
}

#[test]
fn column_masks_preserve_both_face_directions_and_unaligned_tails() {
    let source = words(600);
    let geometry: Vec<_> = source
        .windows(2)
        .map(|pair| u64::from(pair[0]) << 32 | u64::from(pair[1]))
        .collect();
    let blockers: Vec<_> = geometry.iter().map(|value| value.rotate_left(17)).collect();
    for length in [0, 1, 2, 3, 4, 5, 15, 16, 17, 255, 256] {
        let geometry = &geometry[1..length + 1];
        let blockers = &blockers[1..length + 1];
        for negative in [false, true] {
            for mask in [0, 0xffff, u64::MAX] {
                let expected: Vec<_> = geometry
                    .iter()
                    .zip(blockers)
                    .map(|(&a, &b)| a & !(if negative { b << 1 } else { b >> 1 }) & mask)
                    .collect();
                for backend in backends() {
                    let mut actual = vec![0xfeed; length + 2];
                    columns(
                        backend,
                        geometry,
                        blockers,
                        negative,
                        mask,
                        &mut actual[1..length + 1],
                    );
                    assert_eq!(&actual[1..length + 1], expected);
                    assert_eq!(actual[0], 0xfeed);
                    assert_eq!(actual[length + 1], 0xfeed);
                }
                let mut actual = vec![0; length];
                exposed_columns(geometry, blockers, negative, mask, &mut actual);
                assert_eq!(actual, expected);
            }
        }
    }
}

#[test]
fn paired_light_packing_preserves_independent_byte_shift_semantics() {
    let low: Vec<_> = (0..256_u16)
        .map(|index| [index as u8, (255 - index) as u8])
        .collect();
    let high: Vec<_> = low
        .iter()
        .map(|&[a, b]| [a.wrapping_mul(13), b.wrapping_mul(23)])
        .collect();
    for length in [0, 1, 7, 8, 9, 15, 16, 17, 31, 32, 33, 255] {
        let low = &low[1..length + 1];
        let high = &high[1..length + 1];
        let expected: Vec<_> = low
            .iter()
            .zip(high)
            .map(|(a, b)| [a[0] | b[0] << 4, a[1] | b[1] << 4])
            .collect();
        for backend in backends() {
            let mut actual = vec![[37, 71]; length + 2];
            light(backend, low, high, &mut actual[1..length + 1]);
            assert_eq!(&actual[1..length + 1], expected);
            assert_eq!(actual[0], [37, 71]);
            assert_eq!(actual[length + 1], [37, 71]);
        }
        let mut actual = vec![[0; 2]; length];
        pack_light_pairs(low, high, &mut actual);
        assert_eq!(actual, expected);
    }
}

#[test]
fn uniform_scan_rejects_differences_at_any_vector_boundary() {
    for value in [0_u8, 15, 128, 255] {
        for length in [0, 1, 15, 16, 17, 31, 32, 33, 63, 64, 65, 2048] {
            let mut storage = vec![value; length + 2];
            assert!(all_equal_u8(&storage[1..length + 1], value));
            for backend in backends() {
                assert!(equal(backend, &storage[1..length + 1], value));
            }
            for index in 0..length {
                storage[index + 1] = value.wrapping_add(1);
                assert!(!all_equal_u8(&storage[1..length + 1], value));
                for backend in backends() {
                    assert!(!equal(backend, &storage[1..length + 1], value));
                }
                storage[index + 1] = value;
            }
        }
    }
}
