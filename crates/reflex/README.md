# Frame latency controller

This dependency-free crate owns the Off, On and On + Boost policy, adapter
capability reporting, frame IDs and the native backend contract. Unsupported
adapters use the no-op backend. Renderer and platform calls stay outside this crate.

Cinnabar's Windows Vulkan backend enables `VK_NV_low_latency2` when the driver
advertises the required capabilities, configures the swapchain before creation,
and paces before sampling input. Submission and presentation markers are issued
at their native queue boundaries. Video settings retain the requested mode;
startup and mode-change logs report both the request and the effective policy.
An explicit launch override takes precedence over the persisted selection.

The native overlay and its pinned source and licenses are documented in
`third_party/wgpu-hal/UPSTREAM.md`. Sustained stability, surface recreation,
Boost clock behavior and end-to-end latency benefits still need hardware
validation; enabled pacing alone does not establish a latency improvement.
