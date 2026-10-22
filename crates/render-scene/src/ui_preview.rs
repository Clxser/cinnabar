//! Leased editor/export targets. Immutable image leases prevent reuse during display or readback.

use std::sync::Arc;

use bevy::{
    prelude::Resource,
    render::{
        extract_resource::ExtractResource,
        render_resource::{Texture, TextureDimension, TextureFormat, TextureUsages},
        renderer::RenderDevice,
    },
};

pub struct PreviewTarget {
    device: RenderDevice,
    texture: Texture,
}

impl PreviewTarget {
    /// Admits one encoded-sRGB target; its producer bounds allocation and the UI separately admits page size.
    pub fn new(device: RenderDevice, texture: Texture) -> Option<Self> {
        let size = texture.size();
        let maximum = device.limits().max_texture_dimension_2d;
        (texture.dimension() == TextureDimension::D2
            && size.depth_or_array_layers == 1
            && size.width != 0
            && size.width <= maximum
            && size.height != 0
            && size.height <= maximum
            && texture.sample_count() == 1
            && texture.mip_level_count() == 1
            && texture.format() == TextureFormat::Rgba8UnormSrgb
            && texture
                .usage()
                .contains(TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC))
        .then_some(Self { device, texture })
    }

    /// Retains the allocation while another context records or submits commands referring to it.
    pub fn texture(&self) -> &Texture {
        &self.texture
    }

    /// The receiving context must share this exact device; an adapter match alone is insufficient.
    pub fn device(&self) -> &RenderDevice {
        &self.device
    }

    pub fn size(&self) -> [u32; 2] {
        [self.texture.width(), self.texture.height()]
    }
}

pub struct PreviewImage {
    pub target: Arc<PreviewTarget>,
}

#[derive(Clone)]
pub struct UiPreviewImage {
    pub image: Arc<PreviewImage>,
    /// Resolved native page, admitted against the current UI publication before issuing a copy.
    pub page: u32,
    pub placeholder_identity: [u8; 32],
}

#[derive(Clone, Default, Resource, ExtractResource)]
#[extract_app(bevy::render::RenderApp)]
pub struct UiPreviewSurface(pub Option<UiPreviewImage>);
