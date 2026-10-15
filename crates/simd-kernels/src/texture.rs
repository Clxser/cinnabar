use crate::scalar;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
use crate::{Backend, selected_backend, x86};

/// Alpha coverage admitted by an opaque or binary-cutout material.
#[derive(Clone, Copy, Debug)]
pub enum AlphaPolicy {
    Opaque,
    Binary,
}

/// Counts complete RGBA8 pixels whose alpha reaches `threshold`; ignores trailing partial pixels.
pub fn count_alpha_at_least(rgba: &[u8], threshold: u8) -> usize {
    let rgba = &rgba[..rgba.len() / 4 * 4];
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: CPU features are checked; the slice contains only complete pixels.
    unsafe {
        match selected_backend() {
            Backend::Avx2 => return x86::count_alpha_avx2(rgba, threshold),
            Backend::Sse2 => return x86::count_alpha_sse2(rgba, threshold),
            Backend::Scalar => (),
        }
    }
    scalar::count_alpha(rgba, threshold)
}

/// Returns whether complete pixels include both covered and uncovered alpha values.
/// Stops at the first change from the first pixel's coverage; empty input returns false.
pub fn mixed_alpha_coverage(rgba: &[u8], threshold: u8) -> bool {
    let rgba = &rgba[..rgba.len() / 4 * 4];
    if rgba.is_empty() {
        return false;
    }
    let covered = rgba[3] >= threshold;
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: CPU support is checked; all pixels are complete and vector loads are bounded.
    unsafe {
        match selected_backend() {
            Backend::Avx2 => return x86::mixed_alpha_avx2(rgba, threshold, covered),
            Backend::Sse2 => return x86::mixed_alpha_sse2(rgba, threshold, covered),
            Backend::Scalar => (),
        }
    }
    scalar::mixed_alpha(rgba, threshold, covered)
}

/// Tests complete pixels against the policy; empty and trailing partial pixels follow iterator semantics.
pub fn rgba_alpha_matches(rgba: &[u8], policy: AlphaPolicy) -> bool {
    let rgba = &rgba[..rgba.len() / 4 * 4];
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: CPU features are checked; the slice contains only complete pixels.
    unsafe {
        match selected_backend() {
            Backend::Avx2 => return x86::alpha_matches_avx2(rgba, policy),
            Backend::Sse2 => return x86::alpha_matches_sse2(rgba, policy),
            Backend::Scalar => (),
        }
    }
    scalar::alpha_matches(rgba, policy)
}

/// Applies rounded fixed-point alpha scaling, clamped to 255, without changing RGB or partial pixels.
/// `fraction_bits` must be in 1..=31 and the maximum alpha product plus rounding must fit u32.
pub fn scale_rgba_alpha(rgba: &mut [u8], scale: u32, fraction_bits: u32) {
    assert!((1..=31).contains(&fraction_bits));
    let rounding = 1_u32 << (fraction_bits - 1);
    assert!(scale <= (u32::MAX - rounding) / u32::from(u8::MAX));
    let length = rgba.len() / 4 * 4;
    let rgba = &mut rgba[..length];
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: arithmetic and pixel boundaries are validated; dispatch checks CPU features.
    unsafe {
        match selected_backend() {
            Backend::Avx2 => return x86::scale_alpha_avx2(rgba, scale, fraction_bits),
            Backend::Sse2 => return x86::scale_alpha_sse2(rgba, scale, fraction_bits),
            Backend::Scalar => (),
        }
    }
    scalar::scale_alpha(rgba, scale, fraction_bits);
}
