# wgpu-hal Vulkan latency integration

Source: [bedrock-mc/wgpu](https://github.com/bedrock-mc/wgpu), commit
`9686723e5deb6341cf9f088515ced3c8e6fe4f29`, package `wgpu-hal` 30.0.1.
The package originates in [gfx-rs/wgpu](https://github.com/gfx-rs/wgpu).

The local overlay adds NVIDIA Vulkan low-latency extension discovery, device
enablement, swapchain latency setup, pacing and frame markers. Its standalone
Cargo manifest supplies workspace values from the pinned upstream package;
Cinnabar patches the related wgpu packages to the same commit in `Cargo.toml`.
The DX12 PCI address decoder also expresses its unchanged 16-bit mask through
`u16::MAX`.

The upstream MIT and Apache-2.0 licenses are preserved in `LICENSE.MIT` and
`LICENSE.APACHE`. Local changes retain the package's MIT OR Apache-2.0 license.
No NVIDIA SDK source or binaries are included; the integration uses Vulkan
extension declarations exposed by Ash.
