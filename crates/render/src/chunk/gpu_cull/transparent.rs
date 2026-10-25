//! Current-frame occlusion for the sorted transparent pass.
//!
//! Right after the late pass builds its pyramid from this frame's opaque depth, one kernel tests
//! every slot's transparent bounds and rewrites a hidden-slot mask. Transparent terrain draws
//! after all opaque terrain, tests depth with `GreaterEqual` and can only make depth nearer, so a
//! box behind the pyramid's farthest depth over its on-screen footprint draws no fragment. The
//! transparent vertex shader moves hidden slots' vertices outside the clip volume; draw order
//! and every visible pixel are unchanged. The mask is bound only for the view and frame whose
//! late pass rewrote it; every other draw binds an all-visible mask.

use super::kernels::{CullKernels, HizPyramid, TransparentMask};
use super::model::CullPhase;
use super::storage::CullStorage;
use crate::chunk::*;

/// `0` keeps transparent terrain unculled, for same-binary comparisons.
const TRANSPARENT_OCCLUSION_ENV: &str = "CINNABAR_TRANSPARENT_OCCLUSION";

/// The hidden-slot mask, the kernel that writes it and the view it is current for.
pub(in crate::chunk) struct TransparentCull {
    enabled: bool,
    /// The mask and the cull storage capacity it covers.
    mask: Option<(TransparentMask, u32)>,
    failed: crate::gpu_allocation::FailedRequests<u32>,
    bind_group: Option<wgpu::BindGroup>,
    /// The view whose late pass may rewrite the mask this frame.
    frame_view: Option<Entity>,
    /// The view whose late pass did rewrite it; only that view's draws read the mask.
    encoded_view: std::sync::OnceLock<Entity>,
    /// Frames whose mask was rewritten, so tests can tell a culled frame from an unculled one.
    #[cfg(test)]
    encodes: std::sync::atomic::AtomicU64,
}

impl TransparentCull {
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            mask: None,
            failed: default(),
            bind_group: None,
            frame_view: None,
            encoded_view: default(),
            #[cfg(test)]
            encodes: default(),
        }
    }

    /// Whether this process tests transparent terrain at all.
    pub(super) fn enabled_from_env() -> bool {
        env_compat::var_os(TRANSPARENT_OCCLUSION_ENV).is_none_or(|value| value != "0")
    }

    /// Turns the test on or off from the next prepared frame.
    #[cfg(test)]
    pub(in crate::chunk) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// The mask the late pass rewrites for [`Self::encoded_view`].
    pub(in crate::chunk) fn mask(&self) -> Option<&TransparentMask> {
        self.mask.as_ref().map(|(mask, _)| mask)
    }

    /// The view whose late pass rewrote the mask this frame; only its draws may read it.
    pub(in crate::chunk) fn encoded_view(&self) -> Option<Entity> {
        self.encoded_view.get().copied()
    }

    /// Frames whose mask the late pass rewrote.
    #[cfg(test)]
    pub(in crate::chunk) fn encodes(&self) -> u64 {
        self.encodes.load(std::sync::atomic::Ordering::Acquire)
    }

    pub(super) fn end_frame(&mut self) {
        self.frame_view = None;
        self.encoded_view.take();
    }

    /// Readies the kernel for `view` when `admitted` and a pyramid exists, rebuilding its
    /// bindings when `rebind` says the storage or pyramid they name changed.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare(
        &mut self,
        device: &RenderDevice,
        kernels: &CullKernels,
        storage: &CullStorage,
        pyramid: Option<&HizPyramid>,
        rebind: bool,
        view: Entity,
        admitted: bool,
    ) {
        self.frame_view = None;
        let Some(pyramid) = pyramid.filter(|_| self.enabled && admitted) else {
            return;
        };
        let capacity = storage.capacity;
        if self
            .mask
            .as_ref()
            .is_none_or(|(_, covered)| *covered != capacity)
        {
            self.mask = None;
            self.bind_group = None;
            let Some(size) = TransparentMask::size(&device.limits(), kernels.workgroup, capacity)
            else {
                return;
            };
            self.mask = self.failed.attempt(Entity::PLACEHOLDER, capacity, || {
                crate::gpu_allocation::checked(
                    device,
                    "terrain transparent hidden slots",
                    u64::from(size[0]) * u64::from(size[1]) * 4,
                    || {
                        (
                            TransparentMask::new(device.wgpu_device(), size, cfg!(test)),
                            capacity,
                        )
                    },
                )
            });
        }
        let Some((mask, _)) = &self.mask else {
            return;
        };
        if rebind || self.bind_group.is_none() {
            self.bind_group = Some(kernels.transparent_bind_group(
                device.wgpu_device(),
                &storage.uniforms[CullPhase::Late as usize],
                storage,
                pyramid,
                mask,
            ));
        }
        self.frame_view = Some(view);
    }

    /// Rewrites the mask from the pyramid the late pass of `view` just built.
    pub(super) fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        kernels: &CullKernels,
        view: Entity,
    ) {
        if self.frame_view != Some(view) {
            return;
        }
        if let (Some((mask, _)), Some(bind_group)) = (&self.mask, &self.bind_group) {
            kernels.encode_transparent(encoder, bind_group, mask, None);
            // Draws of this view read the mask only once this frame's rewrite is recorded.
            let _ = self.encoded_view.set(view);
            #[cfg(test)]
            self.encodes
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        }
    }
}
