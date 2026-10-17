//! The render-world geometry resource: preparing expansion work and draw lists each frame, and
//! recording the expansion before any camera draws.

use std::collections::HashMap;

use bevy::log::warn;
use bevy::math::{DVec3, IVec3, Vec3};
use bevy::prelude::{Entity, FromWorld, Has, Query, Res, ResMut, Resource, World, default};
use bevy::render::camera::ExtractedCamera;
use bevy::render::render_phase::TrackedRenderPass;
use bevy::render::render_resource::{
    BindGroup, BindGroupEntry, Buffer, BufferDescriptor, BufferId, BufferInitDescriptor,
    BufferUsages, ComputePassDescriptor, PipelineCache,
};
use bevy::render::renderer::{RenderAdapter, RenderContext, RenderDevice};
use bevy::render::view::{ExtractedView, RenderVisibleEntities};
use render_scene::{ChunkRenderInstance, ChunkTextureAssets};

use super::block_ids::{ShaderpackBlockIds, material_attribute_words};
use super::draw::{ViewDraws, face_indices};
use super::jobs::{
    GeometryCategory, JOB_KINDS, JobKind, MaterialTables, SectionPlan, plan_section,
};
use super::pipelines::{
    EXPAND_PARAMS_BYTES, EXPAND_WORKGROUP_SIZE, ExpansionPipelines, output_layout, source_bindings,
    source_layout, storage_buffers_per_stage,
};
use super::residency::{FACE_BYTES, Placement, Residency};
use super::vertex::CONSTANT_STREAM_WORDS;
use crate::chunk::shaderpack_sources::{
    PackedChunkBuffers, PackedChunkSources, ResidentSections, terrain_multi_draw_indirect,
};
use crate::shaderpack::atlas::ShaderpackAtlas;

/// Faces expanded per frame outside an origin move, bounding the frame's expansion work.
pub const FRAME_EXPANSION_FACES: u32 = 1 << 18;
/// Default memory budget of the expanded vertex pages.
pub const DEFAULT_GEOMETRY_BUDGET_BYTES: u64 = 1 << 30;

/// Host controls of the geometry adapter, in the render world.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct ShaderpackGeometrySettings {
    /// Expand and track terrain; while false the adapter holds no GPU memory.
    pub enabled: bool,
    /// The pack's `separateAo`: AO goes to the colour's alpha instead of its RGB.
    pub separate_ao: bool,
    /// The pack's `oldLighting`: bake vanilla directional face shading into the colour.
    pub old_lighting: bool,
    /// Bytes the expanded vertex pages may use; sections beyond it wait, undrawn, until
    /// others leave.
    pub budget_bytes: u64,
}

impl Default for ShaderpackGeometrySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            separate_ao: false,
            old_lighting: false,
            budget_bytes: DEFAULT_GEOMETRY_BUDGET_BYTES,
        }
    }
}

/// Everything whose change rewrites every expanded vertex.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ExpansionInputs {
    materials: BufferId,
    model_templates: BufferId,
    biome_tints: BufferId,
    atlas_generation: u64,
    attribute_key: AttributeKey,
    separate_ao: bool,
    old_lighting: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AttributeKey {
    assets: usize,
    revision: u64,
    block_ids: Option<u64>,
}

struct Dispatch {
    kind: JobKind,
    sources: BindGroup,
    output: BindGroup,
    jobs: u32,
}

/// Expansion recorded by [`record_geometry_expansion`]: page copies first, then dispatches.
#[derive(Default)]
struct PreparedExpansion {
    copies: Vec<(Buffer, Buffer, u64)>,
    dispatches: Vec<Dispatch>,
}

/// Iris-ready terrain geometry for shader packs. See the module documentation for the host
/// contract.
#[derive(Resource)]
pub struct ShaderpackGeometry {
    residency: Residency,
    pages: Vec<Buffer>,
    index_buffer: Option<Buffer>,
    index_faces: u32,
    constants: Buffer,
    attributes: Vec<u32>,
    attribute_key: Option<AttributeKey>,
    inputs: Option<ExpansionInputs>,
    views: HashMap<Entity, ViewDraws>,
    resident: ViewDraws,
    multi_draw: bool,
    supported: bool,
    prepared: PreparedExpansion,
    overflow_reported: bool,
}

impl FromWorld for ShaderpackGeometry {
    fn from_world(world: &mut World) -> Self {
        let device = world.resource::<RenderDevice>();
        let adapter = world.resource::<RenderAdapter>();
        let limits = device.limits();
        let draw_mode = terrain_multi_draw_indirect(adapter, device);
        let supported = draw_mode.is_some()
            && JOB_KINDS.iter().all(|kind| {
                storage_buffers_per_stage(*kind) <= limits.max_storage_buffers_per_shader_stage
            });
        let binding_bytes = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        Self {
            residency: Residency::new(binding_bytes, DEFAULT_GEOMETRY_BUDGET_BYTES),
            pages: Vec::new(),
            index_buffer: None,
            index_faces: 0,
            constants: device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("shader-pack constant vertex stream"),
                contents: bytemuck::cast_slice(&CONSTANT_STREAM_WORDS),
                usage: BufferUsages::VERTEX,
            }),
            attributes: Vec::new(),
            attribute_key: None,
            inputs: None,
            views: HashMap::new(),
            resident: ViewDraws::default(),
            multi_draw: draw_mode == Some(true),
            supported,
            prepared: PreparedExpansion::default(),
            overflow_reported: false,
        }
    }
}

impl ShaderpackGeometry {
    /// Whether this device can expand and draw terrain for packs at all.
    pub fn supported(&self) -> bool {
        self.supported
    }

    /// Block position every expanded vertex position is relative to; it moves (re-expanding
    /// all terrain) when the camera strays [`super::residency::ORIGIN_REBASE_DISTANCE`] away.
    pub fn origin(&self) -> Option<IVec3> {
        self.residency.origin
    }

    /// The pack's `chunkOffset` (and the translation to fold into `gl_ModelViewMatrix`) for a
    /// camera at `camera`: the origin relative to the camera, computed in double precision.
    pub fn chunk_offset(&self, camera: DVec3) -> Option<Vec3> {
        self.residency
            .origin
            .map(|origin| (origin.as_dvec3() - camera).as_vec3())
    }

    /// Records `category` for `view` (a render-world view entity with visible chunks), or for
    /// every resident section when `view` is `None` (shadow passes). The pass must already hold
    /// a pipeline built with [`super::vertex_buffer_layouts`] and the program's bind groups.
    /// Returns the draws issued.
    pub fn draw<'w>(
        &'w self,
        pass: &mut TrackedRenderPass<'w>,
        category: GeometryCategory,
        view: Option<Entity>,
    ) -> u32 {
        let Some(index_buffer) = &self.index_buffer else {
            return 0;
        };
        let draws = match view {
            Some(view) => match self.views.get(&view) {
                Some(draws) => draws,
                None => return 0,
            },
            None => &self.resident,
        };
        draws.record(pass, category, &self.pages, index_buffer, &self.constants)
    }

    fn release_all(&mut self) {
        self.residency.clear();
        self.pages.clear();
        self.index_buffer = None;
        self.index_faces = 0;
        self.inputs = None;
        self.views.clear();
        self.resident = ViewDraws::default();
        self.prepared = PreparedExpansion::default();
    }

    /// Creates or grows vertex page `placement.page` to its new capacity, keeping its faces.
    fn grow_page(&mut self, device: &RenderDevice, placement: &Placement) {
        let Some(previous) = placement.grown_from else {
            return;
        };
        let capacity = self.residency.pages[placement.page].faces.capacity();
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("shader-pack terrain vertices"),
            size: u64::from(capacity) * FACE_BYTES,
            usage: BufferUsages::VERTEX
                | BufferUsages::STORAGE
                | BufferUsages::COPY_SRC
                | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if placement.page < self.pages.len() {
            let old = std::mem::replace(&mut self.pages[placement.page], buffer.clone());
            self.prepared
                .copies
                .push((old, buffer, u64::from(previous) * FACE_BYTES));
        } else {
            self.pages.push(buffer);
        }
    }

    fn ensure_index_buffer(&mut self, device: &RenderDevice, faces: u32) {
        if faces <= self.index_faces && self.index_buffer.is_some() {
            return;
        }
        let faces = faces.max(1).next_power_of_two();
        self.index_buffer = Some(device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("shader-pack terrain face indices"),
            contents: bytemuck::cast_slice(&face_indices(faces)),
            usage: BufferUsages::INDEX,
        }));
        self.index_faces = faces;
    }
}

fn source_group(
    device: &RenderDevice,
    cache: &PipelineCache,
    kind: JobKind,
    buffers: &PackedChunkBuffers<'_>,
) -> BindGroup {
    let buffer = |binding| match binding {
        1 => buffers.quads,
        2 => buffers.origins,
        3 => buffers.materials,
        7 => buffers.biome_records,
        8 => buffers.biome_tints,
        12 => buffers.model_templates,
        _ => buffers.geometry_streams,
    };
    let mut entries: Vec<_> = source_bindings(kind)
        .iter()
        .map(|&binding| BindGroupEntry {
            binding,
            resource: buffer(binding).as_entire_binding(),
        })
        .collect();
    entries.push(BindGroupEntry {
        binding: crate::material_shader::BIOME_QUERY_TABLES_BINDING,
        resource: buffers.world_uniform.as_entire_binding(),
    });
    device.create_bind_group(
        "shader-pack expansion sources",
        &cache.get_bind_group_layout(&source_layout(kind)),
        &entries,
    )
}

/// System parameters of [`prepare_shaderpack_geometry`].
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct GeometryInputs<'w> {
    settings: Option<Res<'w, ShaderpackGeometrySettings>>,
    textures: Option<Res<'w, ChunkTextureAssets>>,
    atlas: Option<Res<'w, ShaderpackAtlas>>,
    block_ids: Option<Res<'w, ShaderpackBlockIds>>,
    pipelines: Res<'w, ExpansionPipelines>,
    cache: Res<'w, PipelineCache>,
    device: Res<'w, RenderDevice>,
}

type GeometryViews<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static ExtractedView,
        &'static RenderVisibleEntities,
        Has<ExtractedCamera>,
    ),
>;

/// Plans this frame's expansion and rebuilds every view's draw lists.
pub(crate) fn prepare_shaderpack_geometry(
    mut geometry: ResMut<ShaderpackGeometry>,
    inputs: GeometryInputs,
    sources: PackedChunkSources,
    mut sections: ResidentSections,
    views: GeometryViews,
) {
    geometry.prepared = PreparedExpansion::default();
    let settings = inputs.settings.as_deref().cloned().unwrap_or_default();
    if !settings.enabled || !geometry.supported {
        if !geometry.pages.is_empty() || geometry.residency.queue_len() > 0 {
            geometry.release_all();
        }
        return;
    }
    let geometry = &mut *geometry;
    geometry.residency.set_budget(settings.budget_bytes);
    for removed in sections.take_removed() {
        geometry.residency.remove(removed);
    }
    for changed in sections.changed().collect::<Vec<_>>() {
        geometry.residency.enqueue(changed);
    }
    // Everything below needs the chunk tables, the atlas and compiled kernels.
    let (Some(buffers), Some(textures), Some(atlas)) = (
        sources.buffers(),
        inputs.textures.as_deref(),
        inputs.atlas.as_deref(),
    ) else {
        return;
    };
    let kernels_ready = geometry.pipelines_ready(&inputs.cache, &inputs.pipelines);
    let attribute_key = AttributeKey {
        assets: std::sync::Arc::as_ptr(&textures.assets) as usize,
        revision: textures.revision,
        block_ids: inputs.block_ids.as_deref().map(|ids| ids.revision),
    };
    if geometry.attribute_key.as_ref() != Some(&attribute_key) {
        geometry.attributes = material_attribute_words(
            &textures.assets,
            inputs.block_ids.as_deref().map(|ids| &*ids.block_entity_id),
        );
        geometry.attribute_key = Some(attribute_key.clone());
    }
    let expansion_inputs = ExpansionInputs {
        materials: buffers.materials.id(),
        model_templates: buffers.model_templates.id(),
        biome_tints: buffers.biome_tints.id(),
        atlas_generation: atlas.generation,
        attribute_key,
        separate_ao: settings.separate_ao,
        old_lighting: settings.old_lighting,
    };
    let camera = views
        .iter()
        .find(|(.., camera)| *camera)
        .map(|(_, view, ..)| view.world_from_view.translation().as_dvec3());
    let rebased = camera.is_some_and(|camera| geometry.residency.follow_camera(camera));
    if rebased || geometry.inputs.as_ref() != Some(&expansion_inputs) {
        let resident: Vec<_> = sections.entities().collect();
        geometry.residency.enqueue_all(resident);
        geometry.inputs = Some(expansion_inputs);
    }
    if kernels_ready && let Some(origin) = geometry.residency.origin {
        let budget = if rebased {
            u32::MAX
        } else {
            FRAME_EXPANSION_FACES
        };
        plan_frame(
            geometry, &inputs, &buffers, &sections, textures, atlas, origin, &settings, budget,
        );
    }
    build_draws(geometry, &inputs.device, &sources, &views);
}

impl ShaderpackGeometry {
    fn pipelines_ready(&self, cache: &PipelineCache, pipelines: &ExpansionPipelines) -> bool {
        pipelines
            .pipelines
            .iter()
            .all(|id| cache.get_compute_pipeline(*id).is_some())
    }
}

#[allow(clippy::too_many_arguments, reason = "one frame's planning inputs")]
fn plan_frame(
    geometry: &mut ShaderpackGeometry,
    inputs: &GeometryInputs,
    buffers: &PackedChunkBuffers<'_>,
    sections: &ResidentSections,
    textures: &ChunkTextureAssets,
    atlas: &ShaderpackAtlas,
    origin: IVec3,
    settings: &ShaderpackGeometrySettings,
    budget: u32,
) {
    let tables = MaterialTables {
        materials: textures.assets.materials(),
        model_templates: textures.assets.model_templates(),
        model_quads: textures.assets.model_quads(),
    };
    let device = &*inputs.device;
    // Jobs per (kind, page), with face indices relative to the page.
    let mut jobs: HashMap<(usize, usize), Vec<[u32; 4]>> = HashMap::new();
    let mut planned_faces = 0_u32;
    let mut deferred = Vec::new();
    let mut overflow = false;
    while planned_faces < budget {
        let Some(entity) = geometry.residency.pop() else {
            break;
        };
        let Some(section) = sections.get(entity) else {
            continue;
        };
        let mut plan: SectionPlan = plan_section(&section.geometry, &tables);
        if plan.faces == 0 {
            geometry.residency.remove(entity);
            continue;
        }
        let Some(placement) = geometry.residency.place(entity, &plan) else {
            overflow = true;
            deferred.push(entity);
            continue;
        };
        geometry.grow_page(device, &placement);
        plan.place(placement.faces.start);
        geometry
            .residency
            .commit(entity, &placement, &plan, section.origin);
        for (kind, planned) in plan.jobs.into_iter().enumerate() {
            jobs.entry((kind, placement.page))
                .or_default()
                .extend(planned);
        }
        planned_faces = planned_faces.saturating_add(plan.faces);
    }
    for entity in deferred {
        geometry.residency.defer(entity);
    }
    if overflow && !geometry.overflow_reported {
        warn!(
            "shader-pack terrain exceeds its {} MiB vertex budget; distant sections wait undrawn",
            settings.budget_bytes >> 20
        );
    }
    geometry.overflow_reported |= overflow;
    if jobs.is_empty() {
        return;
    }
    let mut params = [0_u32; (EXPAND_PARAMS_BYTES / 4) as usize];
    params[..3].copy_from_slice(&origin.to_array().map(|axis| axis as u32));
    params[4] = u32::from(settings.separate_ao);
    params[5] = u32::from(settings.old_lighting);
    params[6] = geometry.attributes.len() as u32;
    params[7] = (geometry.attributes.len() / 2) as u32;
    let params = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("shader-pack expansion parameters"),
        contents: bytemuck::cast_slice(&params),
        usage: BufferUsages::UNIFORM,
    });
    let output_layout = inputs.cache.get_bind_group_layout(&output_layout());
    let mut source_groups: [Option<BindGroup>; 3] = Default::default();
    let mut keys: Vec<_> = jobs.keys().copied().collect();
    keys.sort_unstable();
    for (kind_index, page) in keys {
        let planned = &jobs[&(kind_index, page)];
        let kind = JOB_KINDS[kind_index];
        let mut words = geometry.attributes.clone();
        words.extend(planned.iter().flatten());
        let job_buffer = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("shader-pack expansion jobs"),
            contents: bytemuck::cast_slice(&words),
            usage: BufferUsages::STORAGE,
        });
        let output = device.create_bind_group(
            "shader-pack expansion output",
            &output_layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: job_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: geometry.pages[page].as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: atlas.remap.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: params.as_entire_binding(),
                },
            ],
        );
        let sources = source_groups[kind_index]
            .get_or_insert_with(|| source_group(device, &inputs.cache, kind, buffers))
            .clone();
        geometry.prepared.dispatches.push(Dispatch {
            kind,
            sources,
            output,
            jobs: planned.len() as u32,
        });
    }
}

fn build_draws(
    geometry: &mut ShaderpackGeometry,
    device: &RenderDevice,
    sources: &PackedChunkSources,
    views: &GeometryViews,
) {
    let epoch = geometry.residency.epoch;
    let mut max_faces = 0;
    geometry.views.clear();
    for (entity, view, visible, _) in views.iter() {
        let eye = view.world_from_view.translation().as_dvec3();
        let sections = visible
            .get::<ChunkRenderInstance>()
            .into_iter()
            .flat_map(|class| class.iter_visible())
            .map(|(entity, _)| *entity)
            .filter(|entity| !sources.is_hidden(*entity))
            .filter_map(|entity| geometry.residency.sections.get(&entity));
        let mut draws = ViewDraws::build(sections, Some(eye), epoch);
        if draws.commands.is_empty() {
            continue;
        }
        max_faces = max_faces.max(draws.max_faces());
        if geometry.multi_draw {
            draws.upload_indirect(device);
        }
        geometry.views.insert(entity, draws);
    }
    let mut resident = ViewDraws::build(geometry.residency.sections.values(), None, epoch);
    max_faces = max_faces.max(resident.max_faces());
    if geometry.multi_draw {
        resident.upload_indirect(device);
    }
    geometry.resident = resident;
    if max_faces > 0 {
        geometry.ensure_index_buffer(device, max_faces);
    }
}

/// Records this frame's page growth copies and expansion dispatches ahead of camera passes.
pub(crate) fn record_geometry_expansion(
    mut geometry: ResMut<ShaderpackGeometry>,
    pipelines: Res<ExpansionPipelines>,
    cache: Res<PipelineCache>,
    mut context: RenderContext,
) {
    let prepared = std::mem::take(&mut geometry.prepared);
    if prepared.copies.is_empty() && prepared.dispatches.is_empty() {
        return;
    }
    let encoder = context.command_encoder();
    for (old, new, bytes) in &prepared.copies {
        encoder.copy_buffer_to_buffer(old, 0, new, 0, *bytes);
    }
    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
        label: Some("shader-pack terrain expansion"),
        ..default()
    });
    for dispatch in &prepared.dispatches {
        let Some(pipeline) =
            cache.get_compute_pipeline(pipelines.pipelines[dispatch.kind as usize])
        else {
            continue;
        };
        let groups = dispatch.jobs.div_ceil(EXPAND_WORKGROUP_SIZE);
        if groups == 0 {
            continue;
        }
        let x = groups.min(u16::MAX as u32);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &dispatch.sources, &[]);
        pass.set_bind_group(1, &dispatch.output, &[]);
        pass.dispatch_workgroups(x, groups.div_ceil(x), 1);
    }
}
