//! Turns a shader pack into GPU-ready programs on a worker thread.
//!
//! The worker opens the pack, translates the selected dimension, logs every program's
//! diagnostics, plans buffers and passes, and builds pipelines and pack-wide textures with
//! the shared device. Translation takes tens of milliseconds; driver pipeline compilation
//! dominates the load, so the cache keeps the backend pipeline cache (Vulkan) in a private
//! directory, keyed by the pack's content hash, the translation settings and the adapter.

use super::draws::{DrawPipelineInputs, GeometryDraw, draw_pipeline, select_draws};
use super::effects::EffectSettings;
use super::effects::resources::VanillaTextures;
use super::host_pipelines::{HostPipelines, host_pipelines};
use super::pipelines::{
    FormatSupport, Kind, bind_group_layouts, blend_state, color_target, format_kind,
    layout_entries, quad_attributes, quad_layout, shader_modules, storage_format, texture_format,
};
use super::plan::{
    BufferPlan, FramePlan, MAX_COLOR_BUFFERS, PassStep, PlannedProgram, color_buffer_of,
    plan_buffers, plan_frame,
};
use super::resources::bindings::resolve_bindings;
use super::resources::{
    ATLAS_FORMAT, ConstantBuilder, ConstantTextures, ImageSource, ShadowSlots, TextureSource,
    image_formats,
};
use super::shadow_maps::{ShadowMaps, shadow_maps};
use super::uniforms::PackSettings;
use sha2::{Digest, Sha256};
use shaderpack::{
    BlendMode, BufferId, CustomUniform, Environment, ProgramGroup, ProgramKind, ProgramReflection,
    ProgramTranslation, SamplerBinding, SamplerClass, ScaleRule, ShaderPack, StageKind,
    TranslateConfig,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wgpu::{TextureFormat, TextureUsages};

/// Format of the pack's screen, which `final` writes as Minecraft's 8-bit framebuffer.
pub(crate) const SCREEN_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;
/// Bumped when translated output or the cache layout changes, invalidating cached data.
const CACHE_VERSION: u32 = 2;

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
    /// A geometry program: its pipelines are per draw, in [`PackGpu::draws`].
    Geometry,
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

impl GpuProgram {
    /// Bit per colour buffer the program both samples (`colortexN`) and writes as a storage
    /// image (`colorimgN`). Both bind the same side, which OpenGL allows and wgpu forbids
    /// within one pass, so the host samples a copy taken just before the pass.
    pub fn sampled_while_written(&self) -> u32 {
        let bits = |indices: &mut dyn Iterator<Item = u32>| indices.fold(0, |b, i| b | 1 << i);
        let sampled = bits(&mut self.textures.iter().filter_map(|s| match *s {
            TextureSource::Buffer { index, .. } => Some(index),
            _ => None,
        }));
        let written = bits(&mut self.images.iter().filter_map(|s| match *s {
            ImageSource::Buffer(index) => Some(index),
            _ => None,
        }));
        sampled & written
    }
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
    /// One filtering sampler per [`SamplerClass`], in [`SamplerClass::ALL`] order.
    pub filtering: [wgpu::Sampler; SamplerClass::ALL.len()],
    pub comparison: wgpu::Sampler,
    pub zero_uniforms: wgpu::Buffer,
    pub quad: wgpu::Buffer,
    pub host: HostPipelines,
    /// Geometry draws in recording order; empty when the pack has no terrain program, in
    /// which case the Vanilla scene stands in for the gbuffers.
    pub draws: Vec<GeometryDraw>,
    /// `shadowcomp*` compute passes after the shadow pass.
    pub shadowcomp: Vec<PassStep>,
    /// Present when some shadow draw resolved.
    pub shadow: Option<ShadowMaps>,
    /// `mc_Entity` id per Bedrock block state (by sequential id) from `block.properties`.
    pub block_ids: Option<Arc<Vec<i32>>>,
    /// `entityId` and item ids per Bedrock identifier; empty without geometry draws.
    pub actor_ids: Arc<super::actors::ActorIdMaps>,
    /// `separateAo` and `oldLighting` from `shaders.properties`.
    pub separate_ao: bool,
    pub old_lighting: bool,
    /// Pack-wide placeholder bound for the atlas until Cinnabar's atlas is built.
    pub atlas_placeholder: usize,
    /// Pack-wide `lightmap` texture some geometry program samples; rewritten each frame the
    /// world lightmap changes.
    pub lightmap: Option<usize>,
    /// The pack's effect directives (clouds, sun, moon, stars, weather, particle ordering).
    pub effects: EffectSettings,
}

impl PackGpu {
    /// The sampler bound for a reflected sampler binding.
    pub fn sampler(&self, binding: &SamplerBinding) -> &wgpu::Sampler {
        match binding.class() {
            Some(class) => &self.filtering[class as usize],
            None => &self.comparison,
        }
    }
}

/// The wgpu sampler of a [`SamplerClass`]. Linear classes also filter between mip levels,
/// as Iris does for buffers with mipmaps enabled; repeat/nearest blends mip levels like
/// Java's atlas (`NEAREST_MIPMAP_LINEAR`), which single-level custom textures never see.
pub(crate) fn class_sampler(class: SamplerClass) -> wgpu::SamplerDescriptor<'static> {
    let address = if class.clamps() {
        wgpu::AddressMode::ClampToEdge
    } else {
        wgpu::AddressMode::Repeat
    };
    let filter = if class.linear() {
        wgpu::FilterMode::Linear
    } else {
        wgpu::FilterMode::Nearest
    };
    let mipmap = if class == SamplerClass::ClampNearest {
        wgpu::MipmapFilterMode::Nearest
    } else {
        wgpu::MipmapFilterMode::Linear
    };
    wgpu::SamplerDescriptor {
        label: Some(class.glsl_name()),
        address_mode_u: address,
        address_mode_v: address,
        address_mode_w: address,
        mag_filter: filter,
        min_filter: filter,
        mipmap_filter: mipmap,
        ..Default::default()
    }
}

/// Full-screen groups whose programs must all translate for the pack to load. Geometry
/// programs (`gbuffers_*`, `shadow*`) fall back along Iris's chains instead, and
/// `shadowcomp*` compute passes are optional.
fn runs(group: ProgramGroup) -> bool {
    matches!(
        group,
        ProgramGroup::Prepare
            | ProgramGroup::Deferred
            | ProgramGroup::Composite
            | ProgramGroup::Final
    )
}

/// Groups the host draws or dispatches, whose diagnostics are warnings.
fn drawn(group: ProgramGroup) -> bool {
    runs(group)
        || matches!(
            group,
            ProgramGroup::Gbuffers | ProgramGroup::Shadow | ProgramGroup::ShadowComposite
        )
}

/// Logs a program's diagnostics; failures of programs this host runs are warnings.
fn log_program(pack: &str, program: &ProgramTranslation) {
    let required = program.enabled && drawn(program.program.group);
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

/// Opens, translates and builds `path` for `dimension` (a Java dimension id); `vanilla`
/// answers the pack's `minecraft:` texture references.
pub(crate) fn load(
    path: &Path,
    dimension: &str,
    cache_dir: Option<&Path>,
    gpu: &GpuContext,
    vanilla: &VanillaTextures,
) -> Result<PackGpu, String> {
    let pack = ShaderPack::open(path).map_err(|e| e.to_string())?;
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    build(&pack, &name, dimension, cache_dir, gpu, vanilla)
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
    vanilla: &VanillaTextures,
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
    let (block_ids, actor_ids) =
        super::actors::id_tables(pack.id_maps(&config.environment, &config.options), name);
    let effects = EffectSettings::read(
        &translation.properties,
        runnable
            .iter()
            .any(|p| p.program.group == ProgramGroup::Deferred),
        assets::facts::entity::NativeEntityClass::LightningBolt
            .identifier()
            .and_then(|lightning| actor_ids.entity_id(lightning)),
    );
    let (geometry, selected_draws) =
        select_draws(&translation.programs, |kind| kind.drawn_by(&effects));
    let shadowcomp: Vec<&ProgramTranslation> = translation
        .programs
        .iter()
        .filter(|p| p.enabled && p.program.group == ProgramGroup::ShadowComposite)
        .filter(|p| {
            let compute = p.program.kind == ProgramKind::Compute && p.reflection.is_some();
            if !compute {
                bevy::log::warn!(
                    target: "cinnabar::shaderpack",
                    pack = name, program = %p.program.name,
                    "skipped: only translated shadowcomp compute programs run"
                );
            }
            compute
        })
        .collect();
    bevy::log::info!(
        target: "cinnabar::shaderpack",
        pack = name, folder = %config.folder,
        programs = translation.programs.len(), runnable = runnable.len(),
        geometry = ?selected_draws
            .iter()
            .map(|(kind, i)| format!("{kind:?}={}", geometry[*i].program.name))
            .collect::<Vec<_>>(),
        "translated in {:.0?}", started.elapsed()
    );
    // Full-screen programs first (frame-plan indices), then geometry, then shadowcomp.
    let all: Vec<&ProgramTranslation> = runnable
        .iter()
        .chain(&geometry)
        .chain(&shadowcomp)
        .copied()
        .collect();
    let reflections: Vec<&ProgramReflection> =
        all.iter().filter_map(|p| p.reflection.as_ref()).collect();
    let geometry_plans: Vec<PlannedProgram> = geometry
        .iter()
        .filter(|p| p.program.group == ProgramGroup::Gbuffers)
        .filter_map(|p| Some(planned(p, p.reflection.as_ref()?)))
        .collect();
    let frame = plan_frame(&plans, &translation.properties.flips);
    let buffers = plan_buffers(
        &plans,
        &geometry_plans,
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
    let all_consts = all.iter().flat_map(|p| &p.consts);
    let settings = PackSettings::read(all_consts, &translation.properties);

    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let key = content_key(pack, &config);
    let pipeline_cache = PipelineCacheFile::open(gpu, cache_dir, &key);
    let image_formats = image_formats(&translation.properties.images, &reflections, support);
    let mut constants = ConstantBuilder::new(gpu, pack, &translation, vanilla, support);
    let mut depth_compare = [false; 3];
    let mut programs = Vec::with_capacity(all.len());
    // Per geometry program: its pipeline layout, vertex module and fragment stage.
    let mut geometry_stages = vec![];
    for (program, reflection) in all.iter().zip(&reflections) {
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
        let entries = layout_entries(reflection, &formats, support);
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
        if matches!(
            program.program.group,
            ProgramGroup::Gbuffers | ProgramGroup::Shadow
        ) {
            if program.program.group == ProgramGroup::Shadow {
                for buffer in reflection.color_outputs.iter().filter_map(|o| o.buffer) {
                    constants.shadow_color(buffer);
                }
            }
            let vertex = modules
                .into_iter()
                .find(|(s, _)| *s == StageKind::Vertex)
                .map(|(_, m)| m);
            let fragment = stages
                .into_iter()
                .find(|(s, _)| *s == StageKind::Fragment)
                .map(|(_, m)| m);
            let (Some(vertex), Some(fragment)) = (vertex, fragment) else {
                return Err(format!(
                    "{} needs a vertex and a fragment stage",
                    program.program.name
                ));
            };
            geometry_stages.push((programs.len(), pipeline_layout, vertex, fragment));
            programs.push(GpuProgram {
                name: program.program.name.clone(),
                group: program.program.group,
                reflection: (*reflection).clone(),
                pipeline: ProgramPipeline::Geometry,
                layouts,
                textures: bindings.textures,
                images: bindings.images,
                color_targets: vec![],
                scale: None,
            });
            continue;
        }
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
    let shadow_slots = constants.shadow_slots();
    let draws = geometry_draws(
        gpu,
        pipeline_cache.as_ref().map(|c| &c.cache),
        &selected_draws,
        &geometry_stages,
        &programs,
        &translation.properties,
        &effects,
        &buffer_formats,
        &shadow_slots,
    );
    let shadow = draws
        .iter()
        .any(|d| d.kind.shadow())
        .then(|| shadow_maps(gpu, shadow_slots, &translation.buffers));
    let shadowcomp = (runnable.len() + geometry.len()..programs.len())
        .map(|program| PassStep {
            program,
            read_alt: 0,
            writes: vec![],
            mipmaps: vec![],
        })
        .collect();
    let (block_ids, actor_ids) = if draws.is_empty() {
        (None, Arc::default())
    } else {
        (block_ids, actor_ids)
    };
    let atlas_placeholder = constants
        .placeholder(ATLAS_FORMAT, shaderpack::TextureDimension::D2, false)
        .0;
    let flag = |key: &str| translation.properties.get(key) == Some("true");
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
    let filtering = SamplerClass::ALL.map(|class| gpu.device.create_sampler(&class_sampler(class)));
    let comparison = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("shader pack iris_samplerShadow"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        compare: Some(wgpu::CompareFunction::LessEqual),
        ..Default::default()
    });
    let lightmap = constants.lightmap_slot();
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
        draws,
        shadowcomp,
        shadow,
        block_ids,
        actor_ids,
        separate_ao: flag("separateAo"),
        old_lighting: flag("oldLighting"),
        atlas_placeholder,
        lightmap,
        effects,
    })
}

/// Builds every resolved draw's pipeline. Draws whose program cannot bind terrain vertices
/// are dropped (logged); without a solid terrain draw none remain, so the Vanilla scene
/// copy stays in place.
#[allow(
    clippy::too_many_arguments,
    reason = "the pack-wide plan pieces a draw pipeline draws on"
)]
fn geometry_draws(
    gpu: &GpuContext,
    cache: Option<&wgpu::PipelineCache>,
    selected: &[(super::draws::DrawKind, usize)],
    stages: &[(
        usize,
        wgpu::PipelineLayout,
        wgpu::ShaderModule,
        naga::Module,
    )],
    programs: &[GpuProgram],
    properties: &shaderpack::ShaderProperties,
    effects: &EffectSettings,
    buffer_formats: &[Option<TextureFormat>; MAX_COLOR_BUFFERS as usize],
    shadow: &ShadowSlots,
) -> Vec<GeometryDraw> {
    let mut draws = vec![];
    for (kind, selected) in selected {
        let (index, layout, vertex, fragment) = &stages[*selected];
        let program = &programs[*index];
        let target_format = |buffer: u32| -> Option<TextureFormat> {
            if kind.shadow() {
                shadow
                    .color
                    .iter()
                    .find(|(i, ..)| *i == buffer)
                    .map(|(.., format)| *format)
            } else {
                buffer_formats.get(buffer as usize).copied().flatten()
            }
        };
        let inputs = DrawPipelineInputs {
            device: &gpu.device,
            cache,
            layout,
            name: &program.name,
            reflection: &program.reflection,
            vertex,
            fragment,
            properties,
            effects,
            target_format: &target_format,
        };
        match draw_pipeline(*kind, &inputs) {
            Ok((pipeline, outputs)) => draws.push(GeometryDraw {
                kind: *kind,
                program: *index,
                pipeline,
                outputs,
            }),
            Err(error) => bevy::log::warn!(
                target: "cinnabar::shaderpack",
                program = %program.name, draw = ?kind,
                "draw skipped: {error}"
            ),
        }
    }
    if !draws
        .iter()
        .any(|d| d.kind == super::draws::DrawKind::TerrainSolid)
    {
        draws.clear();
    }
    draws
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
