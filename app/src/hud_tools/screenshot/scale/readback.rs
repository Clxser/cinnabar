//! Recording images copy directly through a bounded, reusable GPU transfer pool.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderContext, RenderDevice},
    },
};

use super::super::SizedCaptureConsumer;
use recording_studio::capture::{MAX_READBACK_BYTES, MAX_READBACKS};

pub(super) struct Request {
    pub(super) size: [u32; 2],
    consumers: Mutex<Vec<SizedCaptureConsumer>>,
}

impl Request {
    /// Retains callbacks only while the admitted frame can still produce pixels.
    pub(super) fn new(size: [u32; 2], consumers: Vec<SizedCaptureConsumer>) -> Arc<Self> {
        Arc::new(Self {
            size,
            consumers: Mutex::new(consumers),
        })
    }

    /// Moves the sole readback allocation to the recording owner; concurrent screenshots stay separate.
    pub(super) fn finish(&self, image: Result<Image, String>) {
        let mut consumers =
            std::mem::take(&mut *self.consumers.lock().expect("recording consumers"));
        match image {
            Ok(image) => {
                if let Some(last) = consumers.pop() {
                    for consumer in consumers {
                        consumer(Ok(image.clone()));
                    }
                    last(Ok(image));
                }
            }
            Err(error) => {
                for consumer in consumers {
                    consumer(Err(error.clone()));
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    size: [u32; 2],
    row: usize,
    padded: usize,
    bytes: u64,
}

impl Layout {
    /// Admission includes GPU row padding, independently of the tightly packed encoder quota.
    fn new(size: [u32; 2]) -> Result<Self, String> {
        recording_studio::video::frame_byte_len(size)?;
        let row = size[0]
            .checked_mul(4)
            .ok_or("Recording row size overflowed.")?;
        let padded = row
            .div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            .checked_mul(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            .ok_or("Recording row padding overflowed.")?;
        let bytes = u64::from(padded) * u64::from(size[1]);
        if bytes > MAX_READBACK_BYTES as u64 {
            return Err("Recording transfer exceeded its memory limit.".into());
        }
        Ok(Self {
            size,
            row: row as usize,
            padded: padded as usize,
            bytes,
        })
    }

    /// Strips row padding with one copy of each pixel, without allocating or copying padded bytes.
    fn pixels(self, source: &[u8]) -> Result<Vec<u8>, String> {
        if source.len() as u64 != self.bytes {
            return Err("Recording GPU transfer has invalid storage.".into());
        }
        if self.row == self.padded {
            return Ok(source.to_vec());
        }
        let mut pixels = Vec::with_capacity(self.row * self.size[1] as usize);
        for source in source.chunks_exact(self.padded) {
            pixels.extend_from_slice(&source[..self.row]);
        }
        Ok(pixels)
    }
}

struct Slot {
    buffer: Buffer,
    layout: Layout,
    busy: Arc<AtomicBool>,
}

#[derive(Resource, Default)]
pub(super) struct Pool {
    slots: Vec<Slot>,
}

impl Pool {
    /// Reuses unmapped buffers only; mapped/in-flight ownership is never reclaimed under a callback.
    pub(super) fn reserve(
        &mut self,
        device: &RenderDevice,
        request: Arc<Request>,
        format: TextureFormat,
    ) -> Result<Option<Transfer>, String> {
        let layout = Layout::new(request.size)?;
        if layout.bytes > device.limits().max_buffer_size {
            return Err("Recording transfer exceeds this device's buffer limit.".into());
        }
        let index = if let Some(index) = self
            .slots
            .iter()
            .position(|slot| slot.layout == layout && !slot.busy.load(Ordering::Acquire))
        {
            index
        } else {
            // A changed recording size retires only idle allocations; old in-flight buffers stay charged.
            self.slots
                .retain(|slot| slot.layout == layout || slot.busy.load(Ordering::Acquire));
            if self.slots.len() >= MAX_READBACKS
                || self.slots.iter().map(|slot| slot.layout.bytes).sum::<u64>() + layout.bytes
                    > MAX_READBACK_BYTES as u64
            {
                return Ok(None);
            }
            self.slots.push(Slot {
                buffer: device.create_buffer(&BufferDescriptor {
                    label: Some("recording transfer"),
                    size: layout.bytes,
                    usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                layout,
                busy: Arc::new(AtomicBool::new(false)),
            });
            self.slots.len() - 1
        };
        let slot = &self.slots[index];
        slot.busy.store(true, Ordering::Release);
        Ok(Some(Transfer {
            buffer: slot.buffer.clone(),
            layout,
            format,
            request,
            permit: Permit(Arc::clone(&slot.busy)),
            copied: AtomicBool::new(false),
        }))
    }

    /// Idle recording allocations are released when the scale output is retired.
    pub(super) fn release_idle(&mut self) {
        self.slots.retain(|slot| slot.busy.load(Ordering::Acquire));
    }
}

struct Permit(Arc<AtomicBool>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(super) struct Transfer {
    buffer: Buffer,
    layout: Layout,
    format: TextureFormat,
    request: Arc<Request>,
    permit: Permit,
    copied: AtomicBool,
}

impl Transfer {
    /// Appends the copy after the completed scale blit on the ordinary render command stream.
    pub(super) fn copy(&self, context: &mut RenderContext, texture: &Texture) {
        context.command_encoder().copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.layout.padded as u32),
                    rows_per_image: Some(self.layout.size[1]),
                },
            },
            Extent3d {
                width: self.layout.size[0],
                height: self.layout.size[1],
                depth_or_array_layers: 1,
            },
        );
        self.copied.store(true, Ordering::Release);
    }

    /// Mapping never polls or waits. Pixel unpacking runs on the existing compute worker pool.
    pub(super) fn map(self) {
        if !self.copied.load(Ordering::Acquire) {
            return;
        }
        let buffer = self.buffer.clone();
        let mapping = buffer.clone();
        mapping
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |mapped| {
                bevy::tasks::AsyncComputeTaskPool::get()
                    .spawn(async move {
                        let Self {
                            layout,
                            format,
                            request,
                            permit,
                            ..
                        } = self;
                        let pixels = mapped
                            .map_err(|error| format!("Recording GPU readback failed: {error}"))
                            .and_then(|()| {
                                let mapped = buffer
                                    .slice(..)
                                    .get_mapped_range()
                                    .map_err(|error| error.to_string())?;
                                layout.pixels(&mapped)
                            });
                        buffer.unmap();
                        drop(permit);
                        request.finish(pixels.map(|pixels| {
                            Image::new(
                                Extent3d {
                                    width: layout.size[0],
                                    height: layout.size[1],
                                    depth_or_array_layers: 1,
                                },
                                TextureDimension::D2,
                                pixels,
                                format,
                                RenderAssetUsages::MAIN_WORLD,
                            )
                        }));
                    })
                    .detach();
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padded_transfer_copies_only_current_rows() {
        let layout = Layout::new([3, 2]).unwrap();
        let mut source = vec![99; layout.bytes as usize];
        source[..12].fill(7);
        source[layout.padded..layout.padded + 12].fill(11);
        assert_eq!(
            layout.pixels(&source).unwrap(),
            [vec![7; 12], vec![11; 12]].concat()
        );
        assert!(layout.pixels(&source[..source.len() - 1]).is_err());
    }

    #[test]
    fn transfer_admission_counts_padding_and_rejects_invalid_extents() {
        assert_eq!(Layout::new([64, 2]).unwrap().bytes, 512);
        assert_eq!(Layout::new([65, 2]).unwrap().bytes, 1024);
        assert!(Layout::new([0, 2]).is_err());
        assert!(Layout::new([u32::MAX, 2]).is_err());
    }

    #[test]
    fn abandoned_transfer_releases_its_slot_once() {
        let busy = Arc::new(AtomicBool::new(true));
        let permit = Permit(Arc::clone(&busy));
        assert!(busy.load(Ordering::Acquire));
        drop(permit);
        assert!(!busy.load(Ordering::Acquire));
    }
}
