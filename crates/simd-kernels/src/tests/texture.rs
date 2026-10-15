use super::backends;
use crate::{
    AlphaPolicy, Backend, count_alpha_at_least, rgba_alpha_matches, scalar, scale_rgba_alpha,
};

fn count(backend: Backend, rgba: &[u8], threshold: u8) -> usize {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: backends() checked instruction support; kernels bound all loads by the slice.
    unsafe {
        match backend {
            Backend::Avx2 => return crate::x86::count_alpha_avx2(rgba, threshold),
            Backend::Sse2 => return crate::x86::count_alpha_sse2(rgba, threshold),
            Backend::Scalar => (),
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    let _ = backend;
    scalar::count_alpha(rgba, threshold)
}

fn matches_policy(backend: Backend, rgba: &[u8], policy: AlphaPolicy) -> bool {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: backends() checked instruction support; kernels handle trailing partial pixels.
    unsafe {
        match backend {
            Backend::Avx2 => return crate::x86::alpha_matches_avx2(rgba, policy),
            Backend::Sse2 => return crate::x86::alpha_matches_sse2(rgba, policy),
            Backend::Scalar => (),
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    let _ = backend;
    scalar::alpha_matches(rgba, policy)
}

fn mixed(backend: Backend, rgba: &[u8], threshold: u8) -> bool {
    let Some(&alpha) = rgba.get(3) else {
        return false;
    };
    let covered = alpha >= threshold;
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: backends() checked support; vector kernels read only complete pixels.
    unsafe {
        match backend {
            Backend::Avx2 => return crate::x86::mixed_alpha_avx2(rgba, threshold, covered),
            Backend::Sse2 => return crate::x86::mixed_alpha_sse2(rgba, threshold, covered),
            Backend::Scalar => (),
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    let _ = backend;
    scalar::mixed_alpha(rgba, threshold, covered)
}

fn scale(backend: Backend, rgba: &mut [u8], multiplier: u32, bits: u32) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: backends() checked support; test arithmetic is within the public kernel's bounds.
    unsafe {
        match backend {
            Backend::Avx2 => return crate::x86::scale_alpha_avx2(rgba, multiplier, bits),
            Backend::Sse2 => return crate::x86::scale_alpha_sse2(rgba, multiplier, bits),
            Backend::Scalar => (),
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    let _ = backend;
    scalar::scale_alpha(rgba, multiplier, bits);
}

#[test]
fn alpha_counts_and_policies_match_for_all_alphas_and_vector_tails() {
    let mut storage = vec![11, 23, 37];
    storage.extend((0..=255_u8).flat_map(|alpha| [alpha.wrapping_add(7), 93, 255, alpha]));
    storage.extend([129, 129, 129]);
    for length in [0, 1, 3, 4, 7, 15, 16, 17, 31, 32, 33, 63, 127, 1024, 1027] {
        let rgba = &storage[3..3 + length];
        for threshold in [0, 1, 127, 128, 254, 255] {
            let expected = rgba
                .chunks_exact(4)
                .filter(|pixel| pixel[3] >= threshold)
                .count();
            assert_eq!(count_alpha_at_least(rgba, threshold), expected);
            let expected_mixed = expected != 0 && expected != rgba.len() / 4;
            assert_eq!(crate::mixed_alpha_coverage(rgba, threshold), expected_mixed);
            for backend in backends() {
                assert_eq!(mixed(backend, rgba, threshold), expected_mixed);
                assert_eq!(
                    count(backend, rgba, threshold),
                    expected,
                    "{backend:?}, length {length}"
                );
            }
        }
        for policy in [AlphaPolicy::Opaque, AlphaPolicy::Binary] {
            let expected = scalar::alpha_matches(rgba, policy);
            assert_eq!(rgba_alpha_matches(rgba, policy), expected);
            for backend in backends() {
                assert_eq!(matches_policy(backend, rgba, policy), expected);
            }
        }
    }
    for policy in [AlphaPolicy::Opaque, AlphaPolicy::Binary] {
        let alpha = if matches!(policy, AlphaPolicy::Opaque) {
            255
        } else {
            0
        };
        let mut rgba = [93, 129, 177, alpha].repeat(35);
        for backend in backends() {
            assert!(matches_policy(backend, &rgba, policy));
        }
        rgba[34 * 4 + 3] = 128;
        for backend in backends() {
            assert!(!matches_policy(backend, &rgba, policy));
        }
    }
    for alpha in [0, 127, 128, 255] {
        let rgba = [23, 41, 71, alpha].repeat(37);
        for threshold in [0, 128, 255] {
            assert!(!crate::mixed_alpha_coverage(&rgba, threshold));
            for backend in backends() {
                assert!(!mixed(backend, &rgba, threshold));
            }
        }
    }
}

#[test]
fn fixed_point_scaling_is_exact_and_preserves_rgb_and_partial_pixels() {
    let mut storage = vec![71, 83, 97];
    storage.extend((0..=255_u8).flat_map(|alpha| [alpha, alpha.wrapping_add(11), 203, alpha]));
    storage.extend([131, 149, 167, 181]);
    for length in [0, 3, 4, 15, 16, 17, 31, 32, 33, 1024, 1027] {
        for bits in [1, 8, 16, 31] {
            let limit = (u32::MAX - (1 << (bits - 1))) / 255;
            for multiplier in [0, 1, 65536, 1048576, limit] {
                let mut expected = storage.clone();
                for pixel in expected[3..3 + length].chunks_exact_mut(4) {
                    pixel[3] = ((u64::from(pixel[3]) * u64::from(multiplier)
                        + (1_u64 << (bits - 1)))
                        >> bits)
                        .min(255) as u8;
                }
                for backend in backends() {
                    let mut actual = storage.clone();
                    scale(backend, &mut actual[3..3 + length], multiplier, bits);
                    assert_eq!(
                        actual, expected,
                        "{backend:?}, length {length}, scale {multiplier}, bits {bits}"
                    );
                }
                let mut actual = storage.clone();
                scale_rgba_alpha(&mut actual[3..3 + length], multiplier, bits);
                assert_eq!(actual, expected);
            }
        }
    }
}
