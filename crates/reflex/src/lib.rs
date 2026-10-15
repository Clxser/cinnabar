//! Vendor-neutral frame-latency control.
//!
//! The controller keeps the pacing contract independent from the renderer.  A backend can supply
//! the native NVIDIA low-latency calls, while unsupported adapters use the same no-op path and
//! retain stable frame markers.  Enabling `VK_NV_low_latency2` and adding its swapchain pNext
//! must happen while the Vulkan device and swapchain are created; the wgpu integration boundary
//! supplies that backend when it exposes those hooks.

mod backend;
mod policy;
mod runtime;

pub use backend::{Backend, BackendError, FrameId, Marker, NoopBackend};
pub use policy::{AdapterIdentity, BackendKind, Capabilities, CapabilityState, Mode};
pub use runtime::Runtime;
