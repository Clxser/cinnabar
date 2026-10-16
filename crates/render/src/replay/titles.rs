//! Bounded GPU ink cache and screen-space title composition, shared by preview and export.

use std::{collections::VecDeque, sync::Arc};

use bevy::render::renderer::RenderDevice;
use render_model::text_bitmap::{MAX_TEXT_CACHE_BYTES, MAX_TEXT_LAYERS, TextBitmap};
use render_scene::ui_preview::PreviewImage;

use super::{ReplayRenderContext, targets::Targets};

pub struct TitleLayer {
    pub bitmap: Arc<TextBitmap>,
    /// The centre of the ink in normalized output coordinates; titles clip at the output edges.
    pub anchor: [f32; 2],
    pub opacity: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
    opacity: f32,
}

struct Cached {
    bitmap: Arc<TextBitmap>,
    group: wgpu::BindGroup,
}

pub struct FrameTitles {
    context: ReplayRenderContext,
    pipeline: Option<Pipeline>,
    cache: VecDeque<Cached>,
    bytes: usize,
    targets: Option<([u32; 2], Targets)>,
}

impl FrameTitles {
    /// No texture or pipeline is allocated until a frame actually contains visible title ink.
    pub fn new(context: ReplayRenderContext) -> Self {
        Self {
            context,
            pipeline: None,
            cache: VecDeque::new(),
            bytes: 0,
            targets: None,
        }
    }

    /// Composites titles in timeline order. None means every output allocation is still leased.
    pub fn render(
        &mut self,
        source: Arc<PreviewImage>,
        layers: &[TitleLayer],
    ) -> Result<Option<Arc<PreviewImage>>, String> {
        if layers.is_empty() {
            return Ok(Some(source));
        }
        if layers.len() > MAX_TEXT_LAYERS
            || layers
                .iter()
                .try_fold(0usize, |bytes, layer| {
                    bytes.checked_add(layer.bitmap.pixels().len())
                })
                .is_none_or(|bytes| bytes > MAX_TEXT_CACHE_BYTES)
        {
            return Err(
                "Too much simultaneous title ink. Shorten overlapping titles or reduce their size."
                    .into(),
            );
        }
        if layers.iter().any(|layer| {
            !layer.anchor.into_iter().all(f32::is_finite)
                || !layer.opacity.is_finite()
                || !(0.0..=1.0).contains(&layer.opacity)
        }) {
            return Err("A title has invalid screen coordinates or opacity.".into());
        }
        if let Some(error) = self.context.faults.error() {
            return Err(error);
        }
        if source.target.device().wgpu_device() != self.context.device.wgpu_device() {
            return Err("The title input belongs to another graphics device.".into());
        }
        let size = source.target.size();
        if self.targets.as_ref().is_none_or(|(old, _)| *old != size) {
            self.targets = Some((size, Targets::new(&self.context.device, size)?));
        }
        let Some(target) = self
            .targets
            .as_mut()
            .expect("title targets admitted")
            .1
            .acquire(&self.context.device)?
        else {
            return Ok(None);
        };
        if self.pipeline.is_none() {
            self.pipeline = Some(Pipeline::new(&self.context.device)?);
        }
        let mut groups = Vec::with_capacity(layers.len());
        let mut vertices = Vec::with_capacity(layers.len() * 6);
        for layer in layers {
            groups.push(self.ink(&layer.bitmap)?);
            let [width, height] = layer.bitmap.size();
            let x = layer.anchor[0] * 2.0 - 1.0;
            let y = 1.0 - layer.anchor[1] * 2.0;
            let dx = width as f32 / size[0] as f32;
            let dy = height as f32 / size[1] as f32;
            for uv in [
                [0.0, 0.0],
                [0.0, 1.0],
                [1.0, 0.0],
                [1.0, 0.0],
                [0.0, 1.0],
                [1.0, 1.0],
            ] {
                vertices.push(Vertex {
                    position: [x + (uv[0] * 2.0 - 1.0) * dx, y - (uv[1] * 2.0 - 1.0) * dy],
                    uv,
                    opacity: layer.opacity,
                });
            }
        }
        let pipeline = self.pipeline.as_ref().expect("title pipeline admitted");
        self.context
            .queue
            .write_buffer(&pipeline.vertices, 0, bytemuck::cast_slice(&vertices));
        let mut encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Studio title composition"),
                });
        encoder.copy_texture_to_texture(
            source.target.texture().as_image_copy(),
            target.texture().as_image_copy(),
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        let output = target.texture().create_view(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Studio screen titles"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_vertex_buffer(0, pipeline.vertices.slice(..));
            for (index, group) in groups.iter().enumerate() {
                pass.set_bind_group(0, group, &[]);
                let start = index as u32 * 6;
                pass.draw(start..start + 6, 0..1);
            }
        }
        self.context.queue.submit([encoder.finish()]);
        let submitted = Arc::clone(&target);
        self.context.queue.on_submitted_work_done(move || {
            drop(submitted);
            drop(source);
        });
        Ok(Some(Arc::new(PreviewImage { target })))
    }

    /// Bitmap pointer identity remains valid because cache entries retain their immutable source allocation.
    fn ink(&mut self, bitmap: &Arc<TextBitmap>) -> Result<wgpu::BindGroup, String> {
        if let Some(index) = self
            .cache
            .iter()
            .position(|entry| Arc::ptr_eq(&entry.bitmap, bitmap))
        {
            let entry = self.cache.remove(index).expect("cached ink index");
            let group = entry.group.clone();
            self.cache.push_back(entry);
            return Ok(group);
        }
        let size = bitmap.size();
        if size
            .into_iter()
            .any(|side| side > self.context.device.limits().max_texture_dimension_2d)
        {
            return Err("Title ink exceeds this GPU's texture dimensions.".into());
        }
        while self.bytes + bitmap.pixels().len() > MAX_TEXT_CACHE_BYTES
            || self.cache.len() >= MAX_TEXT_LAYERS
        {
            let old = self.cache.pop_front().expect("bounded nonempty ink cache");
            self.bytes -= old.bitmap.pixels().len();
        }
        let raw = self.context.device.wgpu_device();
        let texture = raw.create_texture(&wgpu::TextureDescriptor {
            label: Some("Studio cached title ink"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.context.queue.write_texture(
            texture.as_image_copy(),
            bitmap.pixels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 4),
                rows_per_image: Some(size[1]),
            },
            texture.size(),
        );
        let pipeline = self.pipeline.as_ref().expect("title pipeline admitted");
        let group = raw.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Studio title ink"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &texture.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&pipeline.sampler),
                },
            ],
        });
        self.bytes += bitmap.pixels().len();
        self.cache.push_back(Cached {
            bitmap: bitmap.clone(),
            group: group.clone(),
        });
        Ok(group)
    }
}

struct Pipeline {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
}

impl Pipeline {
    /// The checked shader samples already-premultiplied linear ink; blending occurs in linear output space.
    fn new(device: &RenderDevice) -> Result<Self, String> {
        let raw = device.wgpu_device();
        let validation = raw.push_error_scope(wgpu::ErrorFilter::Validation);
        let layout = raw.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Studio title layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader = device.create_and_validate_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Studio titles"),
            source: wgpu::ShaderSource::Wgsl(include_str!("titles.wgsl").into()),
        });
        let pipeline_layout = raw.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Studio title pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attributes = [
            wgpu::VertexAttribute {
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
                offset: std::mem::offset_of!(Vertex, position) as u64,
            },
            wgpu::VertexAttribute {
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x2,
                offset: std::mem::offset_of!(Vertex, uv) as u64,
            },
            wgpu::VertexAttribute {
                shader_location: 2,
                format: wgpu::VertexFormat::Float32,
                offset: std::mem::offset_of!(Vertex, opacity) as u64,
            },
        ];
        let pipeline = raw.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Studio title overlay"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let vertices = raw.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Studio title vertices"),
            size: (MAX_TEXT_LAYERS * 6 * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = raw.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Studio title sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        if let Some(error) = bevy::tasks::block_on(validation.pop()) {
            return Err(format!("Studio title shader preparation failed: {error}"));
        }
        Ok(Self {
            layout,
            sampler,
            pipeline,
            vertices,
        })
    }
}
