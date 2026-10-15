//! CPU kernel selection before chunk and pack workers start.

const SIMD_ENV: &str = "CINNABAR_SIMD";

/// Selects automatic CPU dispatch, with a scalar override for diagnostic comparisons.
pub(super) fn configure() {
    let enabled = env_compat::var(SIMD_ENV).map_or(true, |value| value != "0");
    simd_kernels::set_simd_enabled(enabled);
    diagnostics::log_stderr!(
        "CPU integer kernels: {:?}",
        simd_kernels::selected_backend()
    );
}
