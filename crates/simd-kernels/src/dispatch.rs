use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};

/// CPU instruction set selected for the bounded integer kernels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Scalar,
    Sse2,
    Avx2,
}

static ENABLED: AtomicBool = AtomicBool::new(true);
static DETECTED: OnceLock<Backend> = OnceLock::new();

/// Applies a process-wide diagnostic override before worker startup; detection remains cached.
pub fn set_simd_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// Returns the detected CPU backend, or the scalar backend when explicitly disabled.
pub fn selected_backend() -> Backend {
    if !ENABLED.load(Ordering::Relaxed) {
        return Backend::Scalar;
    }
    *DETECTED.get_or_init(detect)
}

fn detect() -> Backend {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("avx2") {
            return Backend::Avx2;
        }
        if std::is_x86_feature_detected!("sse2") {
            return Backend::Sse2;
        }
    }
    Backend::Scalar
}
