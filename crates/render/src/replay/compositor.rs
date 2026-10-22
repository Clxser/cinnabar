//! Ordered native/raster layers share linear premultiplied composition and leased output targets.

use super::{ReplayRenderContext, targets::Targets};
use bevy::render::render_resource::{Texture, TextureView, TextureViewDescriptor};
use recording_studio::{
    Id, decode::DecodedFrame, framing::FrameTransform, model::Framing, playback::MAX_VISUAL_LAYERS,
    preview::MAX_VISUAL_FRAME_BYTES,
};
use render_scene::ui_preview::PreviewImage;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

mod pipelines;
#[cfg(test)]
mod tests;

pub enum CompositeInput {
    Native(Arc<PreviewImage>),
    Raster(DecodedFrame),
}

pub struct CompositeLayer {
    /// Clip identity keeps overlapping instances of one media source independently cached.
    pub clip: Id,
    pub input: CompositeInput,
    pub framing: Framing,
    pub mix: Option<Id>,
    /// Output-relative image motion; native layers use identity because their camera already moved.
    pub motion: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Parameters {
    crop: [f32; 4],
    inverse_x: [f32; 4],
    inverse_y: [f32; 4],
    control: [f32; 4],
    motion: [f32; 4],
}
impl Parameters {
    const WGSL: &str = "struct Parameters { crop: vec4<f32>, inverse_x: vec4<f32>, inverse_y: vec4<f32>, control: vec4<f32>, motion: vec4<f32>, };";
}

struct Upload {
    size: [u32; 2],
    texture: Texture,
    pixels: Arc<[u8]>,
}
struct Surfaces {
    size: [u32; 2],
    outputs: Targets,
    canvas: Texture,
    group: Texture,
}

pub struct FrameCompositor {
    context: ReplayRenderContext,
    pipelines: Option<pipelines::Pipelines>,
    surfaces: Option<Surfaces>,
    uploads: BTreeMap<Id, Upload>,
}

impl FrameCompositor {
    /// Device handles are cheap; pipelines and bounded surfaces are created only for actual frames.
    pub fn new(context: ReplayRenderContext) -> Self {
        Self {
            context,
            pipelines: None,
            surfaces: None,
            uploads: BTreeMap::new(),
        }
    }

    /// None applies backpressure when every output texture is still leased by display or readback.
    /// Inputs remain leased through queue completion; preview never reads pixels back from the GPU.
    pub fn render(
        &mut self,
        size: [u32; 2],
        layers: &[CompositeLayer],
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<Arc<PreviewImage>>, String> {
        if cancelled() {
            return Err("Composition cancelled.".into());
        }
        if let Some(error) = self.context.faults.error() {
            return Err(error);
        }
        recording_studio::video::frame_byte_len(size)?;
        if layers.len() > MAX_VISUAL_LAYERS {
            return Err("The frame exceeds the visual layer budget.".into());
        }
        if let [layer] = layers
            && layer.mix.is_none()
            && layer.framing == Framing::default()
            && layer.motion == [1.0, 0.0, 0.0]
            && let CompositeInput::Native(image) = &layer.input
            && image.target.size() == size
            && image.target.device().wgpu_device() == self.context.device.wgpu_device()
        {
            self.uploads.clear();
            return Ok(Some(image.clone()));
        }
        self.prepare(size, layers, cancelled)?;
        let surfaces = self
            .surfaces
            .as_mut()
            .expect("composition surfaces admitted");
        let Some(target) = surfaces.outputs.acquire(&self.context.device)? else {
            return Ok(None);
        };
        let output = target
            .texture()
            .create_view(&TextureViewDescriptor::default());
        let canvas = surfaces
            .canvas
            .create_view(&TextureViewDescriptor::default());
        let group = surfaces
            .group
            .create_view(&TextureViewDescriptor::default());
        let pipeline = self
            .pipelines
            .as_ref()
            .expect("composition pipelines admitted");
        let mut encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Studio ordered composition"),
                });
        pipelines::clear(&mut encoder, &canvas, wgpu::Color::BLACK);
        let mut parameters = Vec::with_capacity(layers.len() * 2 + 1);
        let mut mixed = BTreeSet::new();
        for layer in layers {
            if cancelled() {
                return Err("Composition cancelled.".into());
            }
            if let Some(id) = layer.mix {
                if !mixed.insert(id) {
                    continue;
                }
                pipelines::clear(&mut encoder, &group, wgpu::Color::TRANSPARENT);
                for member in layers.iter().filter(|member| member.mix == Some(id)) {
                    let source = view(member, &self.uploads)?;
                    let settings = parameters_for(
                        member.framing,
                        input_size(&member.input),
                        size,
                        false,
                        member.motion,
                    )?;
                    let index = parameters.len();
                    parameters.push(settings);
                    pipeline.pass(
                        &self.context.device,
                        &mut encoder,
                        &source,
                        &group,
                        pipelines::Blend::Add,
                        index,
                    );
                }
                let index = parameters.len();
                parameters.push(parameters_for(
                    Framing::default(),
                    size,
                    size,
                    true,
                    [1.0, 0.0, 0.0],
                )?);
                pipeline.pass(
                    &self.context.device,
                    &mut encoder,
                    &group,
                    &canvas,
                    pipelines::Blend::Over,
                    index,
                );
            } else {
                let source = view(layer, &self.uploads)?;
                let settings = parameters_for(
                    layer.framing,
                    input_size(&layer.input),
                    size,
                    false,
                    layer.motion,
                )?;
                let index = parameters.len();
                parameters.push(settings);
                pipeline.pass(
                    &self.context.device,
                    &mut encoder,
                    &source,
                    &canvas,
                    pipelines::Blend::Over,
                    index,
                );
            }
        }
        if cancelled() {
            return Err("Composition cancelled.".into());
        }
        let index = parameters.len();
        parameters.push(parameters_for(
            Framing::default(),
            size,
            size,
            true,
            [1.0, 0.0, 0.0],
        )?);
        pipeline.pass(
            &self.context.device,
            &mut encoder,
            &canvas,
            &output,
            pipelines::Blend::Output,
            index,
        );
        pipeline.upload(&self.context.queue, &parameters);
        self.context.queue.submit([encoder.finish()]);
        let submitted = target.clone();
        let held: Vec<_> = layers
            .iter()
            .filter_map(|layer| match &layer.input {
                CompositeInput::Native(image) => Some(image.clone()),
                CompositeInput::Raster(_) => None,
            })
            .collect();
        self.context.queue.on_submitted_work_done(move || {
            drop(held);
            drop(submitted);
        });
        Ok(Some(Arc::new(PreviewImage { target })))
    }

    /// Uploads are bounded by active clip identities and bytes, and queue ordering protects prior draws.
    fn prepare(
        &mut self,
        size: [u32; 2],
        layers: &[CompositeLayer],
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let mut active = BTreeSet::new();
        let mut bytes = 0_usize;
        for layer in layers {
            if cancelled() {
                return Err("Composition cancelled.".into());
            }
            if !active.insert(layer.clip) {
                return Err("A visual clip occurs more than once in the frame.".into());
            }
            parameters_for(
                layer.framing,
                input_size(&layer.input),
                size,
                false,
                layer.motion,
            )?;
            let length = recording_studio::video::frame_byte_len(input_size(&layer.input))?;
            bytes = bytes
                .checked_add(length)
                .filter(|bytes| *bytes <= MAX_VISUAL_FRAME_BYTES)
                .ok_or("Visual layers exceed their combined frame byte budget.")?;
            match &layer.input {
                CompositeInput::Native(image) => {
                    if image.target.device().wgpu_device() != self.context.device.wgpu_device() {
                        return Err("A composed source belongs to another graphics device.".into());
                    }
                }
                CompositeInput::Raster(frame) => {
                    let length = recording_studio::video::frame_byte_len(frame.size)?;
                    if frame.rgba8.len() != length {
                        return Err("A raster layer has invalid pixel storage.".into());
                    }
                }
            }
        }
        self.uploads.retain(|clip, _| {
            layers.iter().any(|layer| {
                layer.clip == *clip && matches!(&layer.input, CompositeInput::Raster(_))
            })
        });
        if self.pipelines.is_none() {
            self.pipelines = Some(pipelines::Pipelines::new(&self.context.device)?);
        }
        if self
            .surfaces
            .as_ref()
            .is_none_or(|surfaces| surfaces.size != size)
        {
            let outputs = Targets::new(&self.context.device, size)?;
            self.surfaces = Some(Surfaces {
                size,
                outputs,
                canvas: pipelines::texture(
                    &self.context.device,
                    size,
                    wgpu::TextureFormat::Rgba16Float,
                    wgpu::TextureUsages::empty(),
                ),
                group: pipelines::texture(
                    &self.context.device,
                    size,
                    wgpu::TextureFormat::Rgba16Float,
                    wgpu::TextureUsages::empty(),
                ),
            });
        }
        for layer in layers {
            if cancelled() {
                return Err("Composition cancelled.".into());
            }
            let CompositeInput::Raster(frame) = &layer.input else {
                continue;
            };
            let replace = self
                .uploads
                .get(&layer.clip)
                .is_none_or(|upload| upload.size != frame.size);
            if replace {
                if frame
                    .size
                    .iter()
                    .any(|side| *side > self.context.device.limits().max_texture_dimension_2d)
                {
                    return Err("A raster layer exceeds this device's texture limit.".into());
                }
                self.uploads.insert(
                    layer.clip,
                    Upload {
                        size: frame.size,
                        texture: pipelines::texture(
                            &self.context.device,
                            frame.size,
                            wgpu::TextureFormat::Rgba8UnormSrgb,
                            wgpu::TextureUsages::COPY_DST,
                        ),
                        pixels: Arc::from([]),
                    },
                );
            }
            let upload = self
                .uploads
                .get_mut(&layer.clip)
                .expect("raster upload admitted");
            if !Arc::ptr_eq(&upload.pixels, &frame.rgba8) {
                self.context.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &upload.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &frame.rgba8,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(frame.size[0] * 4),
                        rows_per_image: Some(frame.size[1]),
                    },
                    wgpu::Extent3d {
                        width: frame.size[0],
                        height: frame.size[1],
                        depth_or_array_layers: 1,
                    },
                );
                upload.pixels = frame.rgba8.clone();
            }
        }
        Ok(())
    }
}

/// Native and decoded media expose the same source-pixel aspect ratio to crop placement.
fn input_size(input: &CompositeInput) -> [u32; 2] {
    match input {
        CompositeInput::Native(image) => image.target.size(),
        CompositeInput::Raster(frame) => frame.size,
    }
}

/// Bindings sample retained textures; the caller keeps native leases until submission completes.
fn view(layer: &CompositeLayer, uploads: &BTreeMap<Id, Upload>) -> Result<TextureView, String> {
    Ok(match &layer.input {
        CompositeInput::Native(image) => image
            .target
            .texture()
            .create_view(&TextureViewDescriptor::default()),
        CompositeInput::Raster(_) => uploads
            .get(&layer.clip)
            .ok_or("A composed raster upload is unavailable.")?
            .texture
            .create_view(&TextureViewDescriptor::default()),
    })
}

/// The domain inverse transform owns rotated Fit/Fill geometry; motion and alpha remain independent.
fn parameters_for(
    framing: Framing,
    source: [u32; 2],
    output: [u32; 2],
    premultiplied: bool,
    motion: [f32; 3],
) -> Result<Parameters, String> {
    if !motion.into_iter().all(f32::is_finite) || motion[0] <= 0.0 {
        return Err("A composed image motion is invalid.".into());
    }
    let transform = FrameTransform::new(framing, source, output)?;
    let moved_x = motion[1] / (output[0] as f32 / output[1] as f32);
    if !moved_x.is_finite() {
        return Err("The composed image motion is out of range.".into());
    }
    Ok(Parameters {
        crop: transform.crop,
        inverse_x: transform.inverse_x,
        inverse_y: transform.inverse_y,
        control: [
            framing.opacity as f32,
            if premultiplied { 1.0 } else { 0.0 },
            1.0 / source[0] as f32,
            1.0 / source[1] as f32,
        ],
        motion: [motion[0], moved_x, motion[2], 0.0],
    })
}
