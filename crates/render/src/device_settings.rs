//! Device requirements shared by the native and browser application hosts.

use bevy::render::settings::WgpuSettings;

/// Applies the renderer's storage limits, allocator policy and the platform's device policy.
pub fn configure(settings: &mut WgpuSettings) {
    #[cfg(target_arch = "wasm32")]
    {
        settings.priority = bevy::render::settings::WgpuSettingsPriority::Functionality;
        settings.backends = Some(wgpu::Backends::BROWSER_WEBGPU);
    }
    // Shipping installs use FXC; a DLL in the launch directory must not change that.
    settings.dx12_shader_compiler = wgpu::Dx12Compiler::Fxc;
    settings.limits.max_storage_buffers_per_shader_stage = settings
        .limits
        .max_storage_buffers_per_shader_stage
        .max(crate::required_vertex_storage_buffers());
    // Bevy's default (`Performance`) sub-allocates from 128–256 MiB blocks the allocator
    // keeps while any allocation in them lives; on a 2 GiB working set that left about
    // 1 GiB reserved but unused, and `MemoryUsage`'s 64 MiB blocks still left a few hundred
    // MiB (one or two 15 MiB targets in a 64 MiB block). Blocks of at most 16 MiB hold a
    // full-screen target with little to spare, and anything larger (shader-pack vertex pages,
    // HDR targets, the chunk arena) gets memory of its own.
    settings.memory_hints = wgpu::MemoryHints::Manual {
        suballocated_device_memory_block_size: (4 << 20)..(16 << 20),
    };
}
