//! Turns a shader pack into GPU-ready programs on a worker thread.
//!
//! The worker opens the pack, translates the selected dimension, logs every program's
//! diagnostics, plans buffers and passes, and builds pipelines and pack-wide textures with
//! the shared device. Translation takes tens of milliseconds; driver pipeline compilation
//! dominates the load, so the cache keeps the backend pipeline cache (Vulkan) in a private
//! directory, keyed by the pack's content hash, the translation settings and the adapter.

use super::pipelines::{
    FormatSupport, Kind, bind_group_layouts, blend_state, color_target, format_kind,
    layout_entries, quad_attributes, quad_layout, shader_modules, storage_format, texture_format,
};
use super::plan::{
    BufferPlan, FramePlan, MAX_COLOR_BUFFERS, PlannedProgram, color_buffer_of, plan_buffers,
    plan_frame,
};
use super::resources::{
    ConstantBuilder, ConstantTextures, ImageSource, TextureSource, image_formats, resolve_bindings,
};
use super::uniforms::PackSettings;
use sha2::{Digest, Sha256};
use shaderpack::{
    BlendMode, BufferId, CustomUniform, Environment, ProgramGroup, ProgramKind, ProgramReflection,
    ProgramTranslation, ScaleRule, ShaderPack, StageKind, TranslateConfig,
};
use std::path::{Path, PathBuf};
use wgpu::{TextureFormat, TextureUsages};

/// Format of the pack's screen, which `final` writes as Minecraft's 8-bit framebuffer.
pub(crate) const SCREEN_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;
/// Bumped when translated output or the cache layout changes, invalidating cached data.
const CACHE_VERSION: u32 = 1;

/// The device a pack is built for.
#[derive(Clone)]
pub(crate) struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter: wgpu::Adapter,
}

impl GpuContext {
    fn support(&self) -> FormatSupport {
        FormatSupport::from_features(self.device.features())
    }

    /// Whether the adapter can use `format` with every usage in `usages`.
    pub(crate) fn supports(&self, format: TextureFormat, usages: TextureUsages) -> bool {
        self.adapter
            .get_texture_format_features(format)
            .allowed_usages
            .contains(usages)
    }
}

/// The GPU pipeline of one program.
pub(crate) enum ProgramPipeline {
    Render(wgpu::RenderPipeline),
    Compute(wgpu::ComputePipeline),
}

/// One runnable program with its pipeline and resolved bindings.
pub(crate) struct GpuProgram {
    pub name: String,
    pub group: ProgramGroup,
    pub reflection: ProgramReflection,
    pub pipeline: ProgramPipeline,
    pub layouts: [Option<wgpu::BindGroupLayout>; 4],
    /// Parallel to `reflection.textures`.
    pub textures: Vec<TextureSource>,
    /// Parallel to `reflection.storage_images`.
    pub images: Vec<ImageSource>,
    pub scale: Option<ScaleRule>,
    /// Per fragment output location: the pipeline has a colour target there.
    pub color_targets: Vec<bool>,
}

/// Host passes around the pack's programs.
pub(crate) struct HostPipelines {
    pub scene_colour: wgpu::RenderPipeline,
    pub scene_depth: wgpu::RenderPipeline,
    pub scene_depth_compare: Option<wgpu::RenderPipeline>,
    /// `present` per view target format.
    pub present: Vec<(TextureFormat, wgpu::RenderPipeline)>,
    /// `downsample` per mipmapped buffer format.
    pub downsample: Vec<(TextureFormat, wgpu::RenderPipeline)>,
    pub depth_parameters: wgpu::Buffer,
    pub linear: wgpu::Sampler,
}

/// A loaded pack, immutable and shared by every view.
pub(crate) struct PackGpu {
    pub name: String,
    pub programs: Vec<GpuProgram>,
    pub frame: FramePlan,
    pub buffers: Vec<BufferPlan>,
    /// Texture format of each planned buffer.
    pub buffer_formats: [Option<TextureFormat>; MAX_COLOR_BUFFERS as usize],
    /// Buffers some program binds as `colorimgN`.
    pub storage_buffers: u32,
    /// Format of the `depthtex*` copies sampled as floats.
    pub depth_format: TextureFormat,
    /// `depthtex0..2` some program samples with the comparison sampler.
    pub depth_compare: [bool; 3],
    pub constants: ConstantTextures,
    pub custom_images: Vec<shaderpack::CustomImage>,
    /// Texture format of each custom image.
    pub image_formats: Vec<TextureFormat>,
    /// Storage-buffer bindings with their byte sizes.
    pub shader_storage: Vec<(u32, u64)>,
    pub settings: PackSettings,
    pub custom_uniforms: Vec<CustomUniform>,
    pub filtering: wgpu::Sampler,
    pub comparison: wgpu::Sampler,
    pub zero_uniforms: wgpu::Buffer,
    pub quad: wgpu::Buffer,
    pub host: HostPipelines,
}

/// Groups whose programs this host runs; gbuffers, shadow and shadowcomp wait for the
/// geometry adapter.
fn runs(group: ProgramGroup) -> bool {
    matches!(
        group,
        ProgramGroup::Prepare
            | ProgramGroup::Deferred
            | ProgramGroup::Composite
            | ProgramGroup::Final
    )
}

/// Logs a program's diagnostics; failures of programs this host runs are warnings.
fn log_program(pack: &str, program: &ProgramTranslation) {
    let required = program.enabled && runs(program.program.group);
    for diagnostic in program.all_diagnostics() {
        let location = diagnostic
            .location
            .as_ref()
            .map(|l| format!("{}:{}", l.file, l.line))
            .unwrap_or_default();
        if required {
            bevy::log::warn!(
                target: "cinnabar::shaderpack",
                pack, program = %program.program.name, %location,
                "{:?}: {}", diagnostic.phase, diagnostic.message
            );
        } else {
            bevy::log::debug!(
                target: "cinnabar::shaderpack",
                pack, program = %program.program.name, %location,
                "{:?}: {}", diagnostic.phase, diagnostic.message
            );
        }
    }
}

/// The pack's content hash with the translation settings: the cache key.
pub(crate) fn content_key(pack: &ShaderPack, config: &TranslateConfig) -> String {
    let mut hash = Sha256::new();
    hash.update(CACHE_VERSION.to_le_bytes());
    hash.update(env!("CARGO_PKG_VERSION").as_bytes());
    for (path, text) in pack.source().files() {
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(path.as_bytes());
        hash.update((text.len() as u64).to_le_bytes());
        hash.update(text.as_bytes());
    }
    hash.update(format!("{config:?}").as_bytes());
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The translation settings for this host.
pub(crate) fn translate_config(pack: &ShaderPack, dimension: &str) -> TranslateConfig {
    let environment = Environment {
        os: if cfg!(target_os = "windows") {
            shaderpack::HostOs::Windows
        } else if cfg!(target_os = "macos") {
            shaderpack::HostOs::MacOs
        } else if cfg!(target_os = "linux") {
            shaderpack::HostOs::Linux
        } else {
            shaderpack::HostOs::Other
        },
        ..Environment::default()
    };
    TranslateConfig {
        folder: pack.dimension_folder(dimension),
        environment,
        ..TranslateConfig::default()
    }
}

/// A backend pipeline cache persisted in `cache_dir`.
struct PipelineCacheFile {
    cache: wgpu::PipelineCache,
    path: PathBuf,
}

impl PipelineCacheFile {
    fn open(gpu: &GpuContext, cache_dir: Option<&Path>, key: &str) -> Option<Self> {
        let dir = cache_dir?;
        if !gpu
            .device
            .features()
            .contains(wgpu::Features::PIPELINE_CACHE)
        {
            return None;
        }
        let adapter = wgpu::util::pipeline_cache_key(&gpu.adapter.get_info())?;
        let path = dir.join(format!("{adapter}-{key}.bin"));
        let data = std::fs::read(&path).ok();
        // SAFETY: the data was written by `get_data` of a cache for the same adapter key;
        // `fallback` replaces data the driver rejects.
        let cache = unsafe {
            gpu.device
                .create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                    label: Some("shader pack pipeline cache"),
                    data: data.as_deref(),
                    fallback: true,
                })
        };
        Some(Self { cache, path })
    }

    fn store(&self) {
        let Some(data) = self.cache.get_data() else {
            return;
        };
        let write = || -> std::io::Result<()> {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let temp = self.path.with_extension("tmp");
            std::fs::write(&temp, &data)?;
            std::fs::rename(&temp, &self.path)
        };
        if let Err(error) = write() {
            bevy::log::debug!(target: "cinnabar::shaderpack", %error, "pipeline cache not saved");
        }
    }
}

/// Destination buffer per fragment output location.
fn outputs_by_location(reflection: &ProgramReflection) -> Vec<Option<u32>> {
    let len = reflection
        .color_outputs
        .iter()
        .map(|o| o.location as usize + 1)
        .max()
        .unwrap_or(0);
    let mut outputs = vec![None; len];
    for output in &reflection.color_outputs {
        outputs[output.location as usize] = output.buffer;
    }
    outputs
}

fn planned(program: &ProgramTranslation, reflection: &ProgramReflection) -> PlannedProgram {
    let mipmaps = program
        .consts
        .iter()
        .filter(|c| c.value.trim() == "true")
        .filter_map(|c| c.name.strip_suffix("MipmapEnabled"))
        .filter_map(|b| match BufferId::parse(b) {
            Some(BufferId::Color(i)) if i < MAX_COLOR_BUFFERS => Some(i),
            _ => None,
        })
        .collect();
    PlannedProgram {
        name: program.program.name.clone(),
        group: program.program.group,
        outputs: if reflection.kind == ProgramKind::Render {
            outputs_by_location(reflection)
        } else {
            vec![]
        },
        reads: reflection
            .textures
            .iter()
            .map(|t| t.name.as_str())
            .chain(reflection.storage_images.iter().map(|i| i.name.as_str()))
            .filter_map(color_buffer_of)
            .collect(),
        mipmaps,
    }
}

/// Opens, translates and builds `path` for `dimension` (a Java dimension id).
pub(crate) fn load(
    path: &Path,
    dimension: &str,
    cache_dir: Option<&Path>,
    gpu: &GpuContext,
) -> Result<PackGpu, String> {
    let pack = ShaderPack::open(path).map_err(|e| e.to_string())?;
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    build(&pack, &name, dimension, cache_dir, gpu)
}

/// The enabled programs this host runs, in frame order, with their plans; an error names
/// every such program that failed to translate.
pub(crate) fn runnable_programs(
    translation: &shaderpack::PackTranslation,
) -> Result<(Vec<&ProgramTranslation>, Vec<PlannedProgram>), String> {
    let mut failed = vec![];
    let mut runnable = vec![];
    let mut plans = vec![];
    for program in &translation.programs {
        if !program.enabled || !runs(program.program.group) {
            continue;
        }
        match &program.reflection {
            Some(reflection) => {
                plans.push(planned(program, reflection));
                runnable.push(program);
            }
            None => failed.push(program.program.name.clone()),
        }
    }
    if failed.is_empty() {
        Ok((runnable, plans))
    } else {
        Err(format!("required programs failed: {}", failed.join(", ")))
    }
}

/// Translates and builds an opened pack.
pub(crate) fn build(
    pack: &ShaderPack,
    name: &str,
    dimension: &str,
    cache_dir: Option<&Path>,
    gpu: &GpuContext,
) -> Result<PackGpu, String> {
    let config = translate_config(pack, dimension);
    let started = std::time::Instant::now();
    let translation = pack.translate(&config);
    for diagnostic in &translation.properties.diagnostics {
        bevy::log::debug!(
            target: "cinnabar::shaderpack",
            pack = name, line = diagnostic.line,
            "shaders.properties: {}", diagnostic.message
        );
    }
    for program in &translation.programs {
        log_program(name, program);
    }
    let (runnable, plans) = runnable_programs(&translation)?;
    bevy::log::info!(
        target: "cinnabar::shaderpack",
        pack = name, folder = %config.folder,
        programs = translation.programs.len(), runnable = runnable.len(),
        "translated in {:.0?}", started.elapsed()
    );
    let reflections: Vec<&ProgramReflection> = runnable
        .iter()
        .filter_map(|p| p.reflection.as_ref())
        .collect();
    let frame = plan_frame(&plans, &translation.properties.flips);
    let buffers = plan_buffers(
        &plans,
        &frame,
        &translation.buffers,
        &translation.properties.buffer_sizes,
    );
    let support = gpu.support();
    let mut buffer_formats = [None; MAX_COLOR_BUFFERS as usize];
    for buffer in &buffers {
        buffer_formats[buffer.index as usize] = Some(texture_format(buffer.format, support));
    }
    let mut storage_buffers = 0u32;
    for reflection in &reflections {
        for image in &reflection.storage_images {
            if let Some(b) = color_buffer_of(&image.name)
                && let Some(format) = buffer_formats[b as usize]
                && gpu.supports(format, TextureUsages::STORAGE_BINDING)
                && storage_format(image.format) == Some(format)
            {
                storage_buffers |= 1 << b;
            }
        }
    }
    let depth_format = if support.float32_filterable {
        TextureFormat::R32Float
    } else {
        bevy::log::warn!(
            target: "cinnabar::shaderpack",
            "device cannot filter 32-bit floats; depth buffers use 16-bit precision"
        );
        TextureFormat::R16Float
    };
    let all_consts = runnable.iter().flat_map(|p| &p.consts);
    let settings = PackSettings::read(all_consts, &translation.properties);

    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let key = content_key(pack, &config);
    let pipeline_cache = PipelineCacheFile::open(gpu, cache_dir, &key);
    let image_formats = image_formats(&translation.properties.images, &reflections, support);
    let mut constants = ConstantBuilder::new(gpu, pack, &translation, support);
    let mut depth_compare = [false; 3];
    let mut programs = Vec::with_capacity(runnable.len());
    for (program, reflection) in runnable.iter().zip(&reflections) {
        let bindings = resolve_bindings(
            &mut constants,
            program.program.group,
            reflection,
            &buffer_formats,
            storage_buffers,
            depth_format,
            &translation.properties.images,
            &image_formats,
        );
        for source in &bindings.textures {
            if let TextureSource::DepthCompare(i) = source {
                depth_compare[*i] = true;
            }
        }
        let formats: Vec<TextureFormat> = bindings.textures.iter().map(|s| s.format()).collect();
        let entries = layout_entries(reflection, &formats);
        let layouts = bind_group_layouts(&gpu.device, &program.program.name, &entries);
        let layout_refs: Vec<Option<&wgpu::BindGroupLayout>> =
            layouts.iter().map(Option::as_ref).collect();
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(&format!("shader pack {}", program.program.name)),
                bind_group_layouts: &layout_refs,
                immediate_size: 0,
            });
        let stages: Vec<(StageKind, naga::Module)> = program
            .stages
            .iter()
            .filter_map(|s| s.result.as_ref().ok())
            .map(|c| (c.stage, c.module.clone()))
            .collect();
        let modules = shader_modules(&gpu.device, &program.program.name, &stages);
        let module = |stage: StageKind| modules.iter().find(|(s, _)| *s == stage).map(|(_, m)| m);
        let entry = |stage: StageKind| {
            reflection
                .entry_points
                .iter()
                .find(|(s, _)| *s == stage)
                .map(|(_, e)| e.as_str())
        };
        let cache = pipeline_cache.as_ref().map(|c| &c.cache);
        let targets = color_targets(
            program.program.group,
            &program.program.name,
            reflection,
            &buffer_formats,
            &translation.properties.blends,
        );
        let pipeline = match reflection.kind {
            ProgramKind::Compute => {
                let Some(module) = module(StageKind::Compute) else {
                    return Err(format!("{} has no compute stage", program.program.name));
                };
                ProgramPipeline::Compute(gpu.device.create_compute_pipeline(
                    &wgpu::ComputePipelineDescriptor {
                        label: Some(&program.program.name),
                        layout: Some(&pipeline_layout),
                        module,
                        entry_point: entry(StageKind::Compute),
                        compilation_options: Default::default(),
                        cache,
                    },
                ))
            }
            ProgramKind::Render => {
                let (Some(vertex), Some(fragment)) =
                    (module(StageKind::Vertex), module(StageKind::Fragment))
                else {
                    return Err(format!(
                        "{} needs a vertex and a fragment stage",
                        program.program.name
                    ));
                };
                let attributes = quad_attributes(reflection);
                ProgramPipeline::Render(gpu.device.create_render_pipeline(
                    &wgpu::RenderPipelineDescriptor {
                        label: Some(&program.program.name),
                        layout: Some(&pipeline_layout),
                        vertex: wgpu::VertexState {
                            module: vertex,
                            entry_point: entry(StageKind::Vertex),
                            compilation_options: Default::default(),
                            buffers: &[Some(quad_layout(&attributes))],
                        },
                        primitive: wgpu::PrimitiveState {
                            topology: wgpu::PrimitiveTopology::TriangleStrip,
                            ..Default::default()
                        },
                        depth_stencil: None,
                        multisample: Default::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: fragment,
                            entry_point: entry(StageKind::Fragment),
                            compilation_options: Default::default(),
                            targets: &targets,
                        }),
                        multiview_mask: None,
                        cache,
                    },
                ))
            }
        };
        programs.push(GpuProgram {
            name: program.program.name.clone(),
            group: program.program.group,
            reflection: (*reflection).clone(),
            pipeline,
            layouts,
            textures: bindings.textures,
            images: bindings.images,
            color_targets: targets.iter().map(Option::is_some).collect(),
            scale: translation
                .properties
                .scales
                .iter()
                .rev()
                .find(|s| s.program == program.program.name)
                .cloned(),
        });
    }
    let mipmap_formats: Vec<TextureFormat> = buffers
        .iter()
        .filter(|b| b.mipmapped)
        .filter_map(|b| buffer_formats[b.index as usize])
        .filter(|f| format_kind(*f) == Kind::Float)
        .collect();
    let host = host_pipelines(
        gpu,
        &buffer_formats,
        depth_format,
        depth_compare,
        &mipmap_formats,
    );
    let shader_storage = storage_sizes(&reflections, &translation.properties);
    let max_uniform = reflections
        .iter()
        .flat_map(|r| &r.uniform_buffers)
        .map(|b| u64::from(b.min_size))
        .max()
        .unwrap_or(16)
        .max(16);
    let zero_uniforms = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shader pack zero uniforms"),
        size: max_uniform.next_multiple_of(16),
        usage: wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: false,
    });
    let quad_bytes = super::pipelines::quad_vertices();
    let quad = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shader pack full-screen quad"),
        size: quad_bytes.len() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    gpu.queue.write_buffer(&quad, 0, &quad_bytes);
    let filtering = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("shader pack iris_sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let comparison = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("shader pack iris_samplerShadow"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        compare: Some(wgpu::CompareFunction::LessEqual),
        ..Default::default()
    });
    let constants = constants.finish();
    if let Some(error) = bevy::tasks::block_on(scope.pop()) {
        return Err(format!("the device rejected the pack: {error}"));
    }
    if let Some(cache) = &pipeline_cache {
        cache.store();
    }
    bevy::log::info!(
        target: "cinnabar::shaderpack",
        pack = name, "ready after {:.0?}", started.elapsed()
    );
    Ok(PackGpu {
        name: name.to_string(),
        programs,
        frame,
        buffers,
        buffer_formats,
        storage_buffers,
        depth_format,
        depth_compare,
        constants,
        custom_images: translation.properties.images.clone(),
        image_formats,
        shader_storage,
        settings,
        custom_uniforms: translation.properties.custom_uniforms.clone(),
        filtering,
        comparison,
        zero_uniforms,
        quad,
        host,
    })
}

/// Colour targets per fragment output location. `final` writes the screen at location 0.
fn color_targets(
    group: ProgramGroup,
    name: &str,
    reflection: &ProgramReflection,
    buffer_formats: &[Option<TextureFormat>; MAX_COLOR_BUFFERS as usize],
    blends: &[shaderpack::BlendRule],
) -> Vec<Option<wgpu::ColorTargetState>> {
    let len = reflection
        .color_outputs
        .iter()
        .map(|o| o.location as usize + 1)
        .max()
        .unwrap_or(0);
    let mut targets = vec![None; len];
    let program_blend = blends
        .iter()
        .rev()
        .find(|b| b.program == name && b.buffer.is_none())
        .map(|b| b.mode);
    for output in &reflection.color_outputs {
        let format = if group == ProgramGroup::Final {
            (output.location == 0).then_some(SCREEN_FORMAT)
        } else {
            output
                .buffer
                .and_then(|b| buffer_formats.get(b as usize).copied().flatten())
        };
        let Some(format) = format else {
            continue;
        };
        let buffer_blend = output.buffer.and_then(|b| {
            blends
                .iter()
                .rev()
                .find(|r| {
                    r.program == name && r.buffer.as_deref().and_then(color_buffer_of) == Some(b)
                })
                .map(|r| r.mode)
        });
        let mode = buffer_blend.or(program_blend).unwrap_or(BlendMode::Off);
        targets[output.location as usize] = color_target(format, output.ty, blend_state(mode));
    }
    targets
}

/// Storage-buffer sizes: the reflected minimum, raised by `bufferObject.<binding>=<size>`.
fn storage_sizes(
    reflections: &[&ProgramReflection],
    properties: &shaderpack::ShaderProperties,
) -> Vec<(u32, u64)> {
    let mut sizes: Vec<(u32, u64)> = vec![];
    for buffer in reflections.iter().flat_map(|r| &r.storage_buffers) {
        let declared = properties
            .get(&format!("bufferObject.{}", buffer.binding))
            .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
            .unwrap_or(0);
        let size = u64::from(buffer.min_size)
            .max(declared)
            .max(16)
            .next_multiple_of(16);
        match sizes.iter_mut().find(|(b, _)| *b == buffer.binding) {
            Some((_, s)) => *s = (*s).max(size),
            None => sizes.push((buffer.binding, size)),
        }
    }
    sizes
}

fn host_pipelines(
    gpu: &GpuContext,
    buffer_formats: &[Option<TextureFormat>; MAX_COLOR_BUFFERS as usize],
    depth_format: TextureFormat,
    depth_compare: [bool; 3],
    mipmap_formats: &[TextureFormat],
) -> HostPipelines {
    let device = &gpu.device;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("shader pack host passes"),
        source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
    });
    let pipeline = |entry: &str, targets: &[Option<wgpu::ColorTargetState>], depth: bool| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("fullscreen"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: depth.then(|| wgpu::DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets,
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let target = |format: TextureFormat| {
        Some(wgpu::ColorTargetState {
            format,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })
    };
    let colour0 = buffer_formats[0].unwrap_or(TextureFormat::Rgba8Unorm);
    let mut formats: Vec<TextureFormat> = mipmap_formats.to_vec();
    formats.dedup();
    HostPipelines {
        scene_colour: pipeline("scene_colour", &[target(colour0)], false),
        scene_depth: pipeline(
            "scene_depth",
            &[
                target(depth_format),
                target(depth_format),
                target(depth_format),
            ],
            false,
        ),
        scene_depth_compare: depth_compare
            .contains(&true)
            .then(|| pipeline("scene_depth_compare", &[], true)),
        present: [
            render_scene::SCENE_COLOR_FORMAT,
            render_scene::SCENE_HDR_FORMAT,
        ]
        .into_iter()
        .map(|f| (f, pipeline("present", &[target(f)], false)))
        .collect(),
        downsample: formats
            .into_iter()
            .map(|f| (f, pipeline("downsample", &[target(f)], false)))
            .collect(),
        depth_parameters: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shader pack depth parameters"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        linear: device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shader pack host linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        }),
    }
}
