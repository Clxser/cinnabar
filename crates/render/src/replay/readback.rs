//! One bounded export readback; device polling stays with the normal render-world owner.

use std::{
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use bevy::render::render_resource::{
    Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor,
};
use render_scene::ui_preview::PreviewImage;

use super::ReplayRenderContext;

pub struct ReplayReadback {
    context: ReplayRenderContext,
    buffer: Buffer,
    size: [u32; 2],
    row: u32,
    padded: u32,
    pixels: Arc<[u8]>,
}

impl ReplayReadback {
    /// One admitted GPU buffer and CPU pixel allocation are reused throughout a fixed-size export.
    pub fn new(context: ReplayRenderContext, size: [u32; 2]) -> Result<Self, String> {
        let length = recording_studio::video::frame_byte_len(size)?;
        let row = size[0].checked_mul(4).ok_or("Export row size overflow.")?;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer_size = u64::from(padded) * u64::from(size[1]);
        if buffer_size > context.device.limits().max_buffer_size {
            return Err("The export readback exceeds this device's buffer limit.".into());
        }
        let buffer = context.device.create_buffer(&BufferDescriptor {
            label: Some("Studio export readback"),
            size: buffer_size,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Ok(Self {
            context,
            buffer,
            size,
            row,
            padded,
            pixels: vec![0; length].into(),
        })
    }

    /// Copies a completed replay image on its worker. Normal render polls service the map callback.
    pub fn frame(
        &mut self,
        image: Arc<PreviewImage>,
        cancelled: impl Fn() -> bool,
    ) -> Result<Arc<[u8]>, String> {
        let Self {
            context,
            buffer,
            size,
            row,
            padded,
            pixels,
        } = self;
        if context.device.wgpu_device() != image.target.device().wgpu_device()
            || image.target.size() != *size
        {
            return Err("The export image belongs to another device or extent.".into());
        }
        if cancelled() {
            return Err("Export cancelled.".into());
        }
        let mut encoder = context
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("Studio export frame copy"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: image.target.texture(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &*buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(*padded),
                    rows_per_image: Some(size[1]),
                },
            },
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        context.queue.submit([encoder.finish()]);
        context.queue.on_submitted_work_done(move || drop(image));
        let (mapped, completed) = mpsc::sync_channel(1);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = mapped.send(result);
            });
        let started = Instant::now();
        let result =
            loop {
                if cancelled() {
                    break Err("Export cancelled.".into());
                }
                if let Some(error) = context.faults.error() {
                    break Err(error);
                }
                match completed.recv_timeout(Duration::from_millis(10)) {
                    Ok(result) => {
                        break result.map_err(|error| format!("Export readback failed: {error}"));
                    }
                    Err(mpsc::RecvTimeoutError::Timeout)
                        if started.elapsed() < Duration::from_secs(30) => {}
                    Err(_) => break Err(
                        "Export readback did not finish; keep Cinnabar rendering during export."
                            .into(),
                    ),
                }
            };
        if let Err(error) = result {
            buffer.unmap();
            return Err(error);
        }
        let copied = (|| {
            let view = buffer
                .slice(..)
                .get_mapped_range()
                .map_err(|error| error.to_string())?;
            for (destination, source) in Arc::make_mut(pixels)
                .chunks_exact_mut(*row as usize)
                .zip(view.chunks_exact(*padded as usize))
            {
                destination.copy_from_slice(&source[..*row as usize]);
            }
            Ok::<_, String>(())
        })();
        buffer.unmap();
        copied?;
        Ok(Arc::clone(pixels))
    }
}
