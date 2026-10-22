//! Original full-frame passes share one checked layout and bounded uniform upload.

use super::{MAX_VISUAL_LAYERS, Parameters};
use bevy::render::{
    render_resource::{
        Extent3d, Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
    },
    renderer::{RenderDevice, RenderQueue},
};

pub(super) enum Blend {
    Over,
    Add,
    Output,
}
pub(super) struct Pipelines {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    parameters: wgpu::Buffer,
    stride: usize,
    over: wgpu::RenderPipeline,
    add: wgpu::RenderPipeline,
    output: wgpu::RenderPipeline,
}

impl Pipelines {
    /// Shader validation completes on the composition worker before it can publish an output frame.
    pub fn new(device: &RenderDevice) -> Result<Self, String> {
        let raw = device.wgpu_device();
        let validation = raw.push_error_scope(wgpu::ErrorFilter::Validation);
        let layout = raw.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Studio composition layout"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<Parameters>() as u64
                        ),
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_and_validate_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Studio layer composition"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("layers.wgsl")
                    .replace("// STUDIO_PARAMETERS", Parameters::WGSL)
                    .into(),
            ),
        });
        let pipeline_layout = raw.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Studio composition pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let over = pipeline(
            raw,
            &pipeline_layout,
            &shader,
            wgpu::TextureFormat::Rgba16Float,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let output = pipeline(
            raw,
            &pipeline_layout,
            &shader,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            None,
        );
        let additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let add = pipeline(
            raw,
            &pipeline_layout,
            &shader,
            wgpu::TextureFormat::Rgba16Float,
            Some(wgpu::BlendState {
                color: additive,
                alpha: additive,
            }),
        );
        let alignment = device.limits().min_uniform_buffer_offset_alignment as usize;
        let stride = std::mem::size_of::<Parameters>().div_ceil(alignment) * alignment;
        let parameters = raw.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Studio layer parameters"),
            size: ((MAX_VISUAL_LAYERS * 2 + 1) * stride) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = raw.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Studio composition sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        if let Some(error) = bevy::tasks::block_on(validation.pop()) {
            return Err(format!(
                "Studio composition shader preparation failed: {error}"
            ));
        }
        Ok(Self {
            layout,
            sampler,
            parameters,
            stride,
            over,
            add,
            output,
        })
    }

    /// Each recorded draw binds a different aligned row; one upload precedes the entire submission.
    pub fn upload(&self, queue: &RenderQueue, parameters: &[Parameters]) {
        if parameters.is_empty() {
            return;
        }
        let mut bytes = vec![0_u8; parameters.len() * self.stride];
        for (index, parameters) in parameters.iter().enumerate() {
            let value = bytemuck::bytes_of(parameters);
            bytes[index * self.stride..index * self.stride + value.len()].copy_from_slice(value);
        }
        queue.write_buffer(&self.parameters, 0, &bytes);
    }

    /// wgpu retains bind groups and uniforms through submission; the caller retains native image leases.
    pub fn pass(
        &self,
        device: &RenderDevice,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        output: &wgpu::TextureView,
        blend: Blend,
        index: usize,
    ) {
        let group = device
            .wgpu_device()
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Studio composed layer"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.parameters,
                            offset: (index * self.stride) as u64,
                            size: wgpu::BufferSize::new(std::mem::size_of::<Parameters>() as u64),
                        }),
                    },
                ],
            });
        let pipeline = match blend {
            Blend::Over => &self.over,
            Blend::Add => &self.add,
            Blend::Output => &self.output,
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Studio layer composition"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
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
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Clearing a group and the opaque canvas uses render passes with no sampling or transient textures.
pub(super) fn clear(
    encoder: &mut wgpu::CommandEncoder,
    output: &wgpu::TextureView,
    color: wgpu::Color,
) {
    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Studio composition clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: output,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(color),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}

/// Intermediate groups stay linear float; source uploads and published frames use encoded sRGB.
pub(super) fn texture(
    device: &RenderDevice,
    size: [u32; 2],
    format: TextureFormat,
    extra: TextureUsages,
) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: Some("Studio composition surface"),
        size: Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | extra,
        view_formats: &[],
    })
}

/// One shader performs straight-alpha source sampling and premultiplied group presentation.
fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Studio composition blend"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("layer"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
