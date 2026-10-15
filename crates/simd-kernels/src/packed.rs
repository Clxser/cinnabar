use crate::scalar;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
use crate::{Backend, selected_backend, x86};

/// Expands indices that never straddle a u32 word. High padding bits are ignored.
/// Width zero fills zero; unsupported widths and missing words fill `u32::MAX`.
pub fn unpack_indices(words: &[u32], bits: u8, out: &mut [u32]) {
    if bits == 0 {
        out.fill(0);
        return;
    }
    if bits > 16 {
        out.fill(u32::MAX);
        return;
    }
    let per_word = 32 / usize::from(bits);
    let covered = out.len().min(words.len().saturating_mul(per_word));
    out[covered..].fill(u32::MAX);
    let out = &mut out[..covered];
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: feature dispatch is checked and every output has a corresponding source word.
    unsafe {
        match selected_backend() {
            Backend::Avx2 => return x86::unpack_avx2(words, bits, out),
            Backend::Sse2 => return x86::unpack_sse2(words, bits, out),
            Backend::Scalar => (),
        }
    }
    scalar::unpack(words, bits, out);
}

/// Expands packed indices directly into u16 storage without a u32 staging allocation.
/// Width zero fills zero; unsupported widths and missing words fill `u16::MAX`.
pub fn unpack_indices_u16(words: &[u32], bits: u8, out: &mut [u16]) {
    if bits == 0 {
        out.fill(0);
        return;
    }
    if bits > 16 {
        out.fill(u16::MAX);
        return;
    }
    let per_word = 32 / usize::from(bits);
    let covered = out.len().min(words.len().saturating_mul(per_word));
    out[covered..].fill(u16::MAX);
    let out = &mut out[..covered];
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: dispatch checks support; each output index has a corresponding packed word.
    unsafe {
        match selected_backend() {
            Backend::Avx2 => return x86::unpack_u16_avx2(words, bits, out),
            Backend::Sse2 => return x86::unpack_u16_sse2(words, bits, out),
            Backend::Scalar => (),
        }
    }
    scalar::unpack_u16(words, bits, out);
}

/// Returns the maximum of exactly `count` indices, ignoring word and end padding.
/// Returns None for an unsupported width or truncated packed input; zero entries return zero.
pub fn max_packed_index(words: &[u32], bits: u8, count: usize) -> Option<u32> {
    if count == 0 || bits == 0 {
        return Some(0);
    }
    if bits > 16 {
        return None;
    }
    let per_word = 32 / usize::from(bits);
    if words.len() < count.div_ceil(per_word) {
        return None;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: validated width, count, source length and CPU features bound all reads.
    unsafe {
        match selected_backend() {
            Backend::Avx2 => return Some(x86::max_avx2(words, bits, count)),
            Backend::Sse2 => return Some(x86::max_sse2(words, bits, count)),
            Backend::Scalar => (),
        }
    }
    Some(scalar::max_index(words, bits, count))
}

/// Clears existing indices outside the palette to zero, preserving all padding bits.
/// Missing words stay missing. Width zero and widths above 16 leave the input unchanged.
pub fn sanitize_packed_indices(words: &mut [u32], bits: u8, palette_len: usize, count: usize) {
    if bits == 0 || bits > 16 || palette_len >= 1_usize << bits {
        return;
    }
    let count = count.min(words.len().saturating_mul(32 / usize::from(bits)));
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    // SAFETY: validated width, count, source length and CPU features bound all accesses.
    unsafe {
        match selected_backend() {
            Backend::Avx2 => return x86::sanitize_avx2(words, bits, palette_len, count),
            Backend::Sse2 => return x86::sanitize_sse2(words, bits, palette_len, count),
            Backend::Scalar => (),
        }
    }
    scalar::sanitize(words, bits, palette_len, count);
}
