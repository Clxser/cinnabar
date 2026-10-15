//! Exact integer kernels over bounded slices, with runtime CPU feature dispatch.
//!
//! These functions allocate nothing and accept unaligned slices. Architecture-specific loads
//! never read past the slice; incomplete vector batches write only valid output elements.

mod dispatch;
mod packed;
mod scalar;
mod texture;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86;

pub use dispatch::{Backend, selected_backend, set_simd_enabled};
pub use packed::{max_packed_index, sanitize_packed_indices, unpack_indices, unpack_indices_u16};
pub use texture::{
    AlphaPolicy, count_alpha_at_least, mixed_alpha_coverage, rgba_alpha_matches, scale_rgba_alpha,
};

/// Computes `geometry & !(occluders << 1)` for negative faces, or the right-shift form for
/// positive faces, then masks every column. All slices must have the same length.
pub fn exposed_columns(
    geometry: &[u64],
    occluders: &[u64],
    negative: bool,
    mask: u64,
    out: &mut [u64],
) {
    assert_eq!(geometry.len(), occluders.len());
    assert_eq!(geometry.len(), out.len());
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: dispatch verifies CPU support; equal lengths bound every vector access.
    unsafe {
        match selected_backend() {
            Backend::Avx2 => return x86::columns_avx2(geometry, occluders, negative, mask, out),
            Backend::Sse2 => return x86::columns_sse2(geometry, occluders, negative, mask, out),
            Backend::Scalar => (),
        }
    }
    scalar::columns(geometry, occluders, negative, mask, out);
}

/// Packs adjacent Y samples for both light channels: `low | (high << 4)` per byte.
/// Lengths must match. Values above 15 retain the scalar byte-shift semantics.
pub fn pack_light_pairs(low: &[[u8; 2]], high: &[[u8; 2]], out: &mut [[u8; 2]]) {
    assert_eq!(low.len(), high.len());
    assert_eq!(low.len(), out.len());
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: dispatch verifies CPU support; each pair occupies exactly two bytes.
    unsafe {
        match selected_backend() {
            Backend::Avx2 => return x86::light_avx2(low, high, out),
            Backend::Sse2 => return x86::light_sse2(low, high, out),
            Backend::Scalar => (),
        }
    }
    scalar::light(low, high, out);
}

/// Returns whether every byte equals `value`; an empty slice matches.
pub fn all_equal_u8(values: &[u8], value: u8) -> bool {
    // Heterogeneous channels usually reject immediately, before dispatch or a bulk scan.
    let prefix = values.len().min(16);
    if !values[..prefix].iter().all(|&entry| entry == value) {
        return false;
    }
    let values = &values[prefix..];
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: dispatch verifies CPU support; kernels handle incomplete final vectors.
    unsafe {
        match selected_backend() {
            Backend::Avx2 => return x86::equal_avx2(values, value),
            Backend::Sse2 => return x86::equal_sse2(values, value),
            Backend::Scalar => (),
        }
    }
    values.iter().all(|&entry| entry == value)
}

#[cfg(test)]
mod tests;
