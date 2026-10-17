//! Per-view render targets of a loaded pack and the bind groups that read them.
//!
//! Like Enhanced's persistent post targets, everything here lives as long as the view keeps
//! its size and pack: colour buffers (main and, when the plan ping-pongs them, alternate
//! textures with optional mip chains), the geometry depth attachment and the `depthtex*`
//! copies, the screen `final` draws, custom images, storage buffers, per-program and
//! per-draw uniform buffers and every pass's bind groups. A resize or pack change replaces
//! the whole set.

use super::draws::{DrawKind, GeometryDraw};
use super::loader::{PackGpu, ProgramPipeline, SCREEN_FORMAT};
use super::plan::{MAX_COLOR_BUFFERS, PassStep};
use super::resources::{AtlasLayer, ImageSource, TextureSource};
use shaderpack::{ImageSize, ProgramKind};
use std::sync::Arc;
use wgpu::{Device, TextureFormat, TextureUsages, TextureView};

/// One colour buffer's textures.
pub(crate) struct ColorBuffer {
    pub size: [u32; 2],
    textures: Vec<wgpu::Texture>,
    /// Every mip, for sampling; per side.
    pub sampled: Vec<TextureView>,
    /// Mip 0 only, for attachments and storage; per side.
    pub attachment: Vec<TextureView>,
    /// Each mip level alone, per side (empty without mipmaps).
    pub levels: Vec<Vec<TextureView>>,
}

impl ColorBuffer {
    /// Index of the main (`false`) or alternate (`true`) texture; buffers without an
    /// alternate always use main.
    pub(crate) fn side(&self, alternate: bool) -> usize {
        usize::from(alternate && self.textures.len() > 1)
    }

    pub(crate) fn texture(&self, side: usize) -> &wgpu::Texture {
        &self.textures[side]
    }
}

/// A texture with its default view.
pub(crate) struct Target {
    pub texture: wgpu::Texture,
    pub view: TextureView,
}

fn target(
    device: &Device,
    label: &str,
    format: TextureFormat,
    size: [u32; 2],
    usage: TextureUsages,
) -> Target {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    Target {
        texture,
        view,
    }
}

/// Every view-sized resource of a pack.
pub(crate) struct ViewTargets {
    pub screen_size: [u32; 2],
    pub buffers: Vec<Option<ColorBuffer>>,
    pub depth: [Target; 3],
    pub depth_compare: [Option<Target>; 3],
    /// The depth attachment geometry draws test against (OpenGL window depth, bottom-up);
    /// copied into `depthtex*` after the opaque and translucent geometry.
    pub geometry_depth: Option<Target>,
    /// This frame's sky vertices.
    pub sky: Option<wgpu::Buffer>,
    pub screen: Target,
    pub images: Vec<(wgpu::Texture, TextureView)>,
    pub storage: Vec<(u32, wgpu::Buffer)>,
}

impl ViewTargets {
    pub(crate) fn new(device: &Device, pack: &PackGpu, screen_size: [u32; 2]) -> Self {
        let mut buffers: Vec<Option<ColorBuffer>> = (0..MAX_COLOR_BUFFERS).map(|_| None).collect();
        for plan in &pack.buffers {
            let Some(format) = pack.buffer_formats[plan.index as usize] else {
                continue;
            };
            let size = plan.extent.resolve(screen_size);
            let mips = if plan.mipmapped {
                size[0].max(size[1]).max(1).ilog2() + 1
            } else {
                1
            };
            let mut usage = TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC
                | TextureUsages::COPY_DST;
            if pack.storage_buffers & (1 << plan.index) != 0 {
                usage |= TextureUsages::STORAGE_BINDING;
            }
            let sides = if plan.alternate { 2 } else { 1 };
            let textures: Vec<wgpu::Texture> = (0..sides)
                .map(|side| {
                    device.create_texture(&wgpu::TextureDescriptor {
                        label: Some(&format!(
                            "shader pack colortex{}{}",
                            plan.index,
                            if side == 1 { " alternate" } else { "" }
                        )),
                        size: wgpu::Extent3d {
                            width: size[0],
                            height: size[1],
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: mips,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage,
                        view_formats: &[],
                    })
                })
                .collect();
            let level = |texture: &wgpu::Texture, mip: u32| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            };
            buffers[plan.index as usize] = Some(ColorBuffer {
                size,
                sampled: textures
                    .iter()
                    .map(|t| t.create_view(&Default::default()))
                    .collect(),
                attachment: textures.iter().map(|t| level(t, 0)).collect(),
                levels: if mips > 1 {
                    textures
                        .iter()
                        .map(|t| (0..mips).map(|m| level(t, m)).collect())
                        .collect()
                } else {
                    vec![]
                },
                textures,
            });
        }
        let sampled = TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING;
        let depth = std::array::from_fn(|i| {
            target(
                device,
                &format!("shader pack depthtex{i}"),
                pack.depth_format,
                screen_size,
                sampled,
            )
        });
        let depth_compare = std::array::from_fn(|i| {
            pack.depth_compare[i].then(|| {
                target(
                    device,
                    &format!("shader pack depthtex{i} comparison"),
                    TextureFormat::Depth32Float,
                    screen_size,
                    sampled | TextureUsages::COPY_DST,
                )
            })
        });
        let geometry_depth = (!pack.draws.is_empty()).then(|| {
            target(
                device,
                "shader pack geometry depth",
                TextureFormat::Depth32Float,
                screen_size,
                sampled | TextureUsages::COPY_SRC,
            )
        });
        let sky = (!pack.draws.is_empty()).then(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("shader pack sky vertices"),
                size: super::sky::SKY_BUFFER_BYTES,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let screen = target(
            device,
            "shader pack screen",
            SCREEN_FORMAT,
            screen_size,
            sampled,
        );
        let images = pack
            .custom_images
            .iter()
            .zip(&pack.image_formats)
            .map(|(image, format)| {
                let (dimension, size) = match image.size {
                    ImageSize::Relative { width, height } => (
                        wgpu::TextureDimension::D2,
                        [
                            ((screen_size[0] as f32 * width) as u32).max(1),
                            ((screen_size[1] as f32 * height) as u32).max(1),
                            1,
                        ],
                    ),
                    ImageSize::Absolute {
                        width,
                        height,
                        depth: Some(depth),
                    } => (wgpu::TextureDimension::D3, [width, height, depth]),
                    ImageSize::Absolute { width, height, .. } => {
                        (wgpu::TextureDimension::D2, [width, height, 1])
                    }
                };
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(&image.name),
                    size: wgpu::Extent3d {
                        width: size[0].clamp(1, 8192),
                        height: size[1].clamp(1, 8192),
                        depth_or_array_layers: size[2].clamp(1, 2048),
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension,
                    format: *format,
                    usage: TextureUsages::STORAGE_BINDING
                        | TextureUsages::TEXTURE_BINDING
                        | TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = texture.create_view(&Default::default());
                (texture, view)
            })
            .collect();
        let storage = pack
            .shader_storage
            .iter()
            .map(|(binding, size)| {
                (
                    *binding,
                    device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("shader pack storage buffer"),
                        size: *size,
                        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                )
            })
            .collect();
        Self {
            screen_size,
            buffers,
            depth,
            depth_compare,
            geometry_depth,
            sky,
            screen,
            images,
            storage,
        }
    }

    /// The view a texture binding reads during a pass whose reads use `read_alt`.
    fn texture_view<'a>(
        &'a self,
        pack: &'a PackGpu,
        source: TextureSource,
        read_alt: u32,
        atlas: Option<AtlasViews<'a>>,
    ) -> &'a TextureView {
        match source {
            TextureSource::Buffer { index, .. } => {
                let buffer = self.buffers[index as usize]
                    .as_ref()
                    .expect("planned buffers are allocated");
                &buffer.sampled[buffer.side(read_alt & (1 << index) != 0)]
            }
            TextureSource::Depth { index, .. } => &self.depth[index].view,
            TextureSource::DepthCompare(index) => {
                &self.depth_compare[index]
                    .as_ref()
                    .expect("comparison depth is allocated when a program binds it")
                    .view
            }
            TextureSource::Image { index, .. } => &self.images[index].1,
            TextureSource::Constant { index, .. } => &pack.constants.textures[index].view,
            TextureSource::Atlas(layer) => match atlas {
                Some(atlas) => match layer {
                    AtlasLayer::Color => atlas.color,
                    AtlasLayer::Normals => atlas.normals,
                    AtlasLayer::Specular => atlas.specular,
                },
                None => &pack.constants.textures[pack.atlas_placeholder].view,
            },
        }
    }
}

/// Views of Cinnabar's block atlases, bound to geometry programs' `gtexture`, `normals` and
/// `specular`.
#[derive(Clone, Copy)]
pub(crate) struct AtlasViews<'a> {
    pub color: &'a TextureView,
    pub normals: &'a TextureView,
    pub specular: &'a TextureView,
}

/// Builds the bind groups of program `program` for a pass whose buffer reads use
/// `read_alt`.
pub(crate) fn pass_bind_groups<'a>(
    device: &Device,
    pack: &'a PackGpu,
    targets: &'a ViewTargets,
    uniforms: Option<&'a wgpu::Buffer>,
    program: usize,
    read_alt: u32,
    atlas: Option<AtlasViews<'a>>,
) -> [Option<wgpu::BindGroup>; 4] {
    let program = &pack.programs[program];
    let reflection = &program.reflection;
    let mut entries: [Vec<wgpu::BindGroupEntry<'a>>; 4] = Default::default();
    let mut push = |group: u32, binding: u32, resource: wgpu::BindingResource<'a>| {
        if let Some(e) = entries.get_mut(group as usize) {
            e.push(wgpu::BindGroupEntry { binding, resource });
        }
    };
    if let (Some(block), Some(buffer)) = (&reflection.uniforms, uniforms) {
        push(block.group, block.binding, buffer.as_entire_binding());
    }
    for block in &reflection.uniform_buffers {
        push(
            block.group,
            block.binding,
            wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &pack.zero_uniforms,
                offset: 0,
                size: std::num::NonZeroU64::new(u64::from(block.min_size).next_multiple_of(16)),
            }),
        );
    }
    for block in &reflection.storage_buffers {
        if let Some((_, buffer)) = targets.storage.iter().find(|(b, _)| *b == block.binding) {
            push(block.group, block.binding, buffer.as_entire_binding());
        }
    }
    for sampler in &reflection.samplers {
        push(
            sampler.group,
            sampler.binding,
            wgpu::BindingResource::Sampler(pack.sampler(sampler)),
        );
    }
    for (texture, source) in reflection.textures.iter().zip(&program.textures) {
        push(
            texture.group,
            texture.binding,
            wgpu::BindingResource::TextureView(targets.texture_view(
                pack, *source, read_alt, atlas,
            )),
        );
    }
    for (image, source) in reflection.storage_images.iter().zip(&program.images) {
        let view = match *source {
            ImageSource::Buffer(index) => {
                let buffer = targets.buffers[index as usize]
                    .as_ref()
                    .expect("planned buffers are allocated");
                &buffer.attachment[buffer.side(read_alt & (1 << index) != 0)]
            }
            ImageSource::Image(index) => &targets.images[index].1,
            ImageSource::Constant(index) => &pack.constants.textures[index].view,
        };
        push(
            image.group,
            image.binding,
            wgpu::BindingResource::TextureView(view),
        );
    }
    std::array::from_fn(|group| {
        let layout = program.layouts[group].as_ref()?;
        Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&program.name),
            layout,
            entries: &entries[group],
        }))
    })
}

/// One geometry draw's uniform buffer (its own, since draws sharing a program differ in
/// matrices and render stage) and bind groups.
pub(crate) struct DrawBinding {
    pub uniforms: Option<wgpu::Buffer>,
    pub groups: [Option<wgpu::BindGroup>; 4],
}

/// A view's uniform buffer per program and bind groups per pass.
pub(crate) struct ViewBindings {
    pub uniforms: Vec<Option<wgpu::Buffer>>,
    pub prepare: Vec<[Option<wgpu::BindGroup>; 4]>,
    pub post: Vec<[Option<wgpu::BindGroup>; 4]>,
    pub shadowcomp: Vec<[Option<wgpu::BindGroup>; 4]>,
    /// Parallel to `PackGpu::draws`; groups follow the atlas through [`Self::bind_draws`].
    pub draws: Vec<DrawBinding>,
    /// Per mipmapped buffer and side: one bind group per level after the first, reading
    /// the level above.
    pub downsample: Vec<(u32, usize, Vec<wgpu::BindGroup>)>,
    /// Per view target format, as `PackGpu::host.present`.
    pub present: Vec<(TextureFormat, wgpu::BindGroup)>,
}

fn uniform_buffer(device: &Device, name: &str, block: &shaderpack::UniformBlock) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&format!("shader pack {name} uniforms")),
        size: u64::from(block.size).max(16).next_multiple_of(16),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The buffer sides a geometry draw reads: the side it writes (as Iris's gbuffer samplers
/// do), except buffers it also draws into, which read the other side because a pass cannot
/// sample its own attachment.
pub(crate) fn draw_read_alt(pack: &PackGpu, draw: &GeometryDraw) -> u32 {
    let base = match draw.kind {
        DrawKind::Translucent => pack.frame.translucent_alt,
        kind if kind.shadow() => return 0,
        _ => pack.frame.scene_alt,
    };
    let program = &pack.programs[draw.program];
    let mut read_alt = base;
    for buffer in draw.outputs.iter().flatten() {
        let read = program
            .textures
            .iter()
            .any(|t| matches!(t, TextureSource::Buffer { index, .. } if index == buffer));
        if read {
            read_alt ^= 1 << buffer;
        }
    }
    read_alt
}

impl ViewBindings {
    pub(crate) fn new(device: &Device, pack: &Arc<PackGpu>, targets: &ViewTargets) -> Self {
        let uniforms: Vec<Option<wgpu::Buffer>> = pack
            .programs
            .iter()
            .map(|p| {
                let fullscreen = !matches!(p.pipeline, ProgramPipeline::Geometry);
                let block = p.reflection.uniforms.as_ref().filter(|_| fullscreen)?;
                Some(uniform_buffer(device, &p.name, block))
            })
            .collect();
        let groups = |steps: &[PassStep]| {
            steps
                .iter()
                .map(|step| {
                    pass_bind_groups(
                        device,
                        pack,
                        targets,
                        uniforms[step.program].as_ref(),
                        step.program,
                        step.read_alt,
                        None,
                    )
                })
                .collect()
        };
        let prepare = groups(&pack.frame.prepare);
        let post = groups(&pack.frame.post);
        let shadowcomp = groups(&pack.shadowcomp);
        let draws = pack
            .draws
            .iter()
            .map(|draw| {
                let program = &pack.programs[draw.program];
                let uniforms = program
                    .reflection
                    .uniforms
                    .as_ref()
                    .map(|block| uniform_buffer(device, &program.name, block));
                DrawBinding {
                    uniforms,
                    groups: Default::default(),
                }
            })
            .collect();
        let mut downsample = vec![];
        for plan in pack.buffers.iter().filter(|b| b.mipmapped) {
            let Some(buffer) = targets.buffers[plan.index as usize].as_ref() else {
                continue;
            };
            let Some(format) = pack.buffer_formats[plan.index as usize] else {
                continue;
            };
            let Some((_, pipeline)) = pack.host.downsample.iter().find(|(f, _)| *f == format)
            else {
                continue;
            };
            let layout = pipeline.get_bind_group_layout(0);
            for (side, levels) in buffer.levels.iter().enumerate() {
                let groups = levels
                    .iter()
                    .take(levels.len().saturating_sub(1))
                    .map(|source| {
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("shader pack downsample"),
                            layout: &layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: wgpu::BindingResource::TextureView(source),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 3,
                                    resource: wgpu::BindingResource::Sampler(&pack.host.linear),
                                },
                            ],
                        })
                    })
                    .collect();
                downsample.push((plan.index, side, groups));
            }
        }
        let present_source = if pack
            .programs
            .iter()
            .any(|p| p.group == shaderpack::ProgramGroup::Final)
        {
            &targets.screen.view
        } else {
            let buffer = targets.buffers[0]
                .as_ref()
                .expect("colortex0 is always planned");
            &buffer.sampled[0]
        };
        let present = pack
            .host
            .present
            .iter()
            .map(|(format, pipeline)| {
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("shader pack present"),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(present_source),
                    }],
                });
                (*format, group)
            })
            .collect();
        Self {
            uniforms,
            prepare,
            post,
            shadowcomp,
            draws,
            downsample,
            present,
        }
    }

    /// Rebuilds every draw's bind groups over `atlas` (the placeholder while `None`).
    pub(crate) fn bind_draws(
        &mut self,
        device: &Device,
        pack: &PackGpu,
        targets: &ViewTargets,
        atlas: Option<AtlasViews<'_>>,
    ) {
        for (draw, binding) in pack.draws.iter().zip(&mut self.draws) {
            binding.groups = pass_bind_groups(
                device,
                pack,
                targets,
                binding.uniforms.as_ref(),
                draw.program,
                draw_read_alt(pack, draw),
                atlas,
            );
        }
    }
}

/// Whether a program dispatches compute work.
pub(crate) fn is_compute(pack: &PackGpu, step: &PassStep) -> bool {
    pack.programs[step.program].reflection.kind == ProgramKind::Compute
}
