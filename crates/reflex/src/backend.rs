//! Native pacing contract and the unsupported-adapter fallback.

use std::num::NonZeroU64;

use crate::Mode;

/// Stable markers shared by simulation, submission and presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Marker {
    InputSample,
    SimulationStart,
    SimulationEnd,
    SubmissionStart,
    SubmissionEnd,
    PresentationStart,
    PresentationEnd,
}

/// A nonzero ID carried by every marker belonging to one frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FrameId(pub(crate) NonZeroU64);

impl FrameId {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Errors returned by a native pacing backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackendError;

/// Native backend contract for `VK_NV_low_latency2` or another supported latency API.
///
/// Implementations must enable the extension and chain its swapchain latency structure before
/// swapchain creation, then issue the sleep and marker calls for the supplied frame IDs.
pub trait Backend: Send + Sync {
    /// Maps to `vkSetLatencySleepModeNV` after the swapchain latency pNext is installed.
    fn set_latency_sleep_mode(&self, mode: Mode) -> Result<(), BackendError>;
    /// Maps to `vkLatencySleepNV` and its timeline-semaphore wait before input sampling.
    fn latency_sleep(&self, frame: FrameId) -> Result<(), BackendError>;
    /// Maps to `vkSetLatencyMarkerNV` for the supplied frame ID.
    fn set_latency_marker(&self, frame: FrameId, marker: Marker);
}

/// Safe fallback for unsupported GPUs and renderers without a native swapchain hook.
#[derive(Debug, Default)]
pub struct NoopBackend;

impl Backend for NoopBackend {
    fn set_latency_sleep_mode(&self, _mode: Mode) -> Result<(), BackendError> {
        Ok(())
    }

    fn latency_sleep(&self, _frame: FrameId) -> Result<(), BackendError> {
        Ok(())
    }

    fn set_latency_marker(&self, _frame: FrameId, _marker: Marker) {}
}
