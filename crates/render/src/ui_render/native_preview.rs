//! Copies native preview targets before UI draws, retaining each source through GPU completion.

use std::sync::{Arc, Weak};

use bevy::{
    prelude::*,
    render::{
        render_resource::{
            CommandEncoderDescriptor, Extent3d, Origin3d, TexelCopyTextureInfo, TextureId,
        },
        renderer::{RenderDevice, RenderQueue},
    },
};
use render_scene::{
    UiRenderSceneResource,
    ui_preview::{PreviewImage, UiPreviewSurface},
};

use super::{UiGpu, damage::UiDamage};

#[derive(Resource, Default)]
pub(super) struct PreviewCopy {
    source: Weak<PreviewImage>,
    destination: Option<(TextureId, u32)>,
    pending: Option<Arc<PreviewImage>>,
    page: Option<u32>,
}

impl PreviewCopy {
    /// Only batches sampling the copied page need to invalidate their retained UI pixels.
    pub(super) fn damage(&self, batch: &render_model::UiRenderBatch) -> UiDamage {
        if self.page == Some(batch.texture_page) {
            UiDamage::Rect(batch.scissor)
        } else {
            UiDamage::Unchanged
        }
    }
}

/// Page identity, extent, encoding and device admission happen before any cross-context command.
pub(super) fn copy(
    source: Res<UiPreviewSurface>,
    scene: Res<UiRenderSceneResource>,
    gpu: Res<UiGpu>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut state: ResMut<PreviewCopy>,
) {
    state.page = None;
    let Some(source) = &source.0 else {
        state.source = Weak::new();
        state.destination = None;
        return;
    };
    let Some(input) = scene
        .input
        .as_ref()
        .filter(|input| gpu.accepted_revision == Some(input.revision))
    else {
        return;
    };
    let Some(page) = input.textures.pages().get(source.page as usize) else {
        return;
    };
    let target = &source.image.target;
    if page.identity() != source.placeholder_identity
        || page.dimensions() != target.size()
        || page.format() != render_model::UiTextureFormat::Rgba8
        || device.wgpu_device() != target.device().wgpu_device()
        || !gpu.textures.resident(&input.textures)
    {
        return;
    }
    let Some(location) = gpu.textures.locations.get(source.page as usize) else {
        return;
    };
    let Some(bucket) = gpu.textures.buckets.get(location.bucket) else {
        return;
    };
    let destination = (bucket.texture.id(), location.layer);
    if state.source.ptr_eq(&Arc::downgrade(&source.image)) && state.destination == Some(destination)
    {
        return;
    }
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("native Studio preview copy"),
    });
    let [width, height] = target.size();
    encoder.copy_texture_to_texture(
        TexelCopyTextureInfo {
            texture: target.texture(),
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: Default::default(),
        },
        TexelCopyTextureInfo {
            texture: &bucket.texture,
            mip_level: 0,
            origin: Origin3d {
                x: 0,
                y: 0,
                z: location.layer,
            },
            aspect: Default::default(),
        },
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    state.source = Arc::downgrade(&source.image);
    state.destination = Some(destination);
    state.page = Some(source.page);
    // This immediate submission already references the source. Do not wait for a later render system.
    let lease = Arc::clone(&source.image);
    queue.on_submitted_work_done(move || drop(lease));
    state.pending = Some(Arc::clone(&source.image));
}

/// Holds the source past UI render submission as well as the earlier texture copy submission.
pub(super) fn retire(queue: Res<RenderQueue>, mut state: ResMut<PreviewCopy>) {
    if let Some(lease) = state.pending.take() {
        queue.on_submitted_work_done(move || drop(lease));
    }
}
