//! Original Studio post shaders share one checked layout and host uniform contract.

use bevy::render::renderer::RenderDevice;

use super::Parameters;

pub(super) enum Pass {
    Extract,
    Horizontal,
    Grade,
}

pub(super) struct Pipelines {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pub parameters: wgpu::Buffer,
    extract: wgpu::RenderPipeline,
    horizontal: wgpu::RenderPipeline,
    grade: wgpu::RenderPipeline,
}

impl Pipelines {
    /// Every entry is checked before the worker publishes a composed frame.
    pub fn new(device: &RenderDevice) -> Result<Self, String> {
        let raw = device.wgpu_device();
        let validation = raw.push_error_scope(wgpu::ErrorFilter::Validation);
        let layout = raw.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Studio effects layout"),
            entries: &[
                texture(0),
                texture(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
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
        let module = device.create_and_validate_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Studio frame effects"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("effects.wgsl")
                    .replace("// STUDIO_PARAMETERS", Parameters::WGSL)
                    .into(),
            ),
        });
        let pipeline_layout = raw.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Studio effects pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let extract = pipeline(
            raw,
            &pipeline_layout,
            &module,
            "extract",
            wgpu::TextureFormat::Rgba16Float,
        );
        let horizontal = pipeline(
            raw,
            &pipeline_layout,
            &module,
            "horizontal",
            wgpu::TextureFormat::Rgba16Float,
        );
        let grade = pipeline(
            raw,
            &pipeline_layout,
            &module,
            "grade",
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let parameters = raw.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Studio effects parameters"),
            size: std::mem::size_of::<Parameters>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = raw.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Studio effects linear sampler"),
            min_filter: wgpu::FilterMode::Linear,
            mag_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        if let Some(error) = bevy::tasks::block_on(validation.pop()) {
            return Err(format!("Studio effects shader preparation failed: {error}"));
        }
        Ok(Self {
            layout,
            sampler,
            parameters,
            extract,
            horizontal,
            grade,
        })
    }

    /// Bind groups are local command resources; wgpu retains their allocations through submission completion.
    pub fn pass(
        &self,
        device: &RenderDevice,
        encoder: &mut wgpu::CommandEncoder,
        kind: Pass,
        source: &wgpu::TextureView,
        glow: &wgpu::TextureView,
        output: &wgpu::TextureView,
    ) {
        let group = device
            .wgpu_device()
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Studio effects input"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(glow),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.parameters.as_entire_binding(),
                    },
                ],
            });
        let pipeline = match kind {
            Pass::Extract => &self.extract,
            Pass::Horizontal => &self.horizontal,
            Pass::Grade => &self.grade,
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Studio frame effects"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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

/// Both intermediate and original images are filterable 2D float textures with no multisampling.
fn texture(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// A full-screen triangle avoids per-frame vertex buffers or topology updates.
fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    entry: &str,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Studio frame effects"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
