use crate::Backend;

mod masks;
mod packed;
mod texture;

/// Enumerates only instruction sets supported by the machine running the tests.
fn backends() -> Vec<Backend> {
    #[allow(unused_mut)]
    let mut available = vec![Backend::Scalar];
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("sse2") {
            available.push(Backend::Sse2);
        }
        if std::is_x86_feature_detected!("avx2") {
            available.push(Backend::Avx2);
        }
    }
    available
}

/// Produces reproducible bit patterns without a platform RNG or dependency.
fn words(length: usize) -> Vec<u32> {
    let mut state = 0x35ed_017b_u32;
    (0..length)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            state
        })
        .collect()
}
