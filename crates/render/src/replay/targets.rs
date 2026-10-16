//! A small GPU target pool. Display/copy/completion leases all participate in reuse admission.

use std::sync::Arc;

use bevy::render::{
    render_resource::{
        Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
    },
    renderer::RenderDevice,
};
use render_scene::ui_preview::PreviewTarget;

const MAX_TARGETS: usize = 4;

pub(super) struct Targets {
    size: [u32; 2],
    pool: Vec<Arc<PreviewTarget>>,
}

impl Targets {
    /// Bounds both each target's extent and the maximum number of simultaneously leased allocations.
    pub(super) fn new(device: &RenderDevice, size: [u32; 2]) -> Result<Self, String> {
        recording_studio::video::frame_byte_len(size)?;
        if size
            .into_iter()
            .any(|side| side > device.limits().max_texture_dimension_2d)
        {
            return Err("The native replay output exceeds its GPU texture budget.".into());
        }
        Ok(Self {
            size,
            pool: Vec::new(),
        })
    }

    /// Backpressure keeps the worker from overwriting a frame still displayed or referenced by the GPU.
    pub(super) fn acquire(
        &mut self,
        device: &RenderDevice,
    ) -> Result<Option<Arc<PreviewTarget>>, String> {
        if let Some(target) = self
            .pool
            .iter()
            .find(|target| Arc::strong_count(target) == 1)
        {
            return Ok(Some(Arc::clone(target)));
        }
        if self.pool.len() == MAX_TARGETS {
            return Ok(None);
        }
        let [width, height] = self.size;
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("native Studio replay target"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::COPY_SRC
                | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let target = Arc::new(
            PreviewTarget::new(device.clone(), texture).ok_or("The replay target was rejected.")?,
        );
        self.pool.push(Arc::clone(&target));
        Ok(Some(target))
    }
}
