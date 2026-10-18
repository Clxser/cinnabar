//! The render-world geometry resource: preparing expansion work and draw lists each frame, and
//! recording the expansion before any camera draws.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use bevy::log::{debug, warn};
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
use render_scene::{ChunkRenderInstance, ChunkTextureAssets, RuntimeStage, RuntimeStageProfiler};

use super::block_ids::{ShaderpackBlockIds, material_attribute_words};
use super::draw::{ViewDraws, face_indices};
use super::jobs::{
    GeometryCategory, JOB_KINDS, JobKind, MaterialTables, SectionPlan, plan_section,
};
use super::pipelines::{
    EXPAND_PARAMS_BYTES, EXPAND_WORKGROUP_SIZE, ExpansionPipelines, output_layout, source_bindings,
    source_layout, storage_buffers_per_stage,
};
use super::residency::{
    Eviction, FACE_BYTES, Placement, Residency, ResidentFaces, SECTION_RADIUS, section_distance,
};
use super::vertex::CONSTANT_STREAM_WORDS;
use crate::chunk::shaderpack_sources::{
    PackedChunkBuffers, PackedChunkSources, ResidentSections, terrain_multi_draw_indirect,
};
use crate::shaderpack::atlas::ShaderpackAtlas;

/// Faces expanded per frame outside an origin move, bounding the frame's expansion work.
pub const FRAME_EXPANSION_FACES: u32 = 1 << 18;
/// Default memory budget of the expanded vertex pages, the most any device gets.
pub const DEFAULT_GEOMETRY_BUDGET_BYTES: u64 = 1 << 30;
/// Smallest device-sized budget.
const MIN_GEOMETRY_BUDGET_BYTES: u64 = 256 << 20;
/// Budget when the device does not report dedicated memory (shared-memory adapters).
const SHARED_MEMORY_GEOMETRY_BUDGET_BYTES: u64 = 512 << 20;

/// The vertex budget for an adapter with `dedicated` bytes of graphics memory (0 when it
/// reports none): an eighth of it, within 256 MiB and [`DEFAULT_GEOMETRY_BUDGET_BYTES`].
pub fn device_geometry_budget(dedicated: u64) -> u64 {
    if dedicated == 0 {
        return SHARED_MEMORY_GEOMETRY_BUDGET_BYTES;
    }
    (dedicated / 8).clamp(MIN_GEOMETRY_BUDGET_BYTES, DEFAULT_GEOMETRY_BUDGET_BYTES)
}

/// Camera travel, in blocks, after which sections waiting for room are tried again.
const RETRY_DISTANCE: f64 = 16.0;

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
    /// Eye distance within which every visible section is expanded; `None` when all are.
    complete_radius: Option<f32>,
    /// Eye distance of the nearest section the vertex budget left unexpanded at its last
    /// overflow; `None` once no section waits.
    budget_horizon: Option<f32>,
    /// Camera position when waiting sections were last retried.
    retry_eye: Option<DVec3>,
    /// The vertex pages' size when the device last ran out of memory growing them; the
    /// budget stays below it, so nearer sections take room from farther ones instead.
    memory_cap: Option<u64>,
    log: ResidencyLog,
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
            complete_radius: None,
            budget_horizon: None,
            retry_eye: None,
            memory_cap: None,
            log: ResidencyLog::default(),
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

    /// Eye distance within which every section the camera may see is expanded, or `None`
    /// when nothing waits. Beyond it terrain may be missing from the pack's geometry.
    pub fn complete_radius(&self) -> Option<f32> {
        self.complete_radius
    }

    /// Eye distance past which the vertex budget leaves terrain undrawn, `None` while every
    /// section fits. Unlike [`Self::complete_radius`] it ignores sections merely queued for
    /// this frame's expansion, so it holds steady while terrain streams in.
    pub fn budget_horizon(&self) -> Option<f32> {
        self.budget_horizon
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
        self.complete_radius = None;
        self.budget_horizon = None;
        self.retry_eye = None;
        self.memory_cap = None;
    }

    /// Creates or grows vertex page `placement.page` to its new capacity, keeping its faces.
    /// Returns false, leaving the page as it was, when the device is out of memory.
    fn grow_page(&mut self, device: &RenderDevice, placement: &Placement) -> bool {
        let Some(previous) = placement.grown_from else {
            return true;
        };
        let capacity = self.residency.pages[placement.page].faces.capacity();
        let Some(buffer) = crate::gpu_allocation::buffer(
            device,
            &BufferDescriptor {
                label: Some("shader-pack terrain vertices"),
                size: u64::from(capacity) * FACE_BYTES,
                usage: BufferUsages::VERTEX
                    | BufferUsages::STORAGE
                    | BufferUsages::COPY_SRC
                    | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        ) else {
            return false;
        };
        if placement.page < self.pages.len() {
            let old = std::mem::replace(&mut self.pages[placement.page], buffer.clone());
            self.prepared
                .copies
                .push((old, buffer, u64::from(previous) * FACE_BYTES));
        } else {
            self.pages.push(buffer);
        }
        true
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
    /// The pack's `far`, the client render distance in blocks.
    frame: Option<Res<'w, render_scene::shaderpack::ShaderPackFrameInputs>>,
    pipelines: Res<'w, ExpansionPipelines>,
    cache: Res<'w, PipelineCache>,
    device: Res<'w, RenderDevice>,
    profiler: Option<Res<'w, RuntimeStageProfiler>>,
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
    let _timer = inputs
        .profiler
        .as_deref()
        .map(|profiler| profiler.time(RuntimeStage::ShaderpackGeometry));
    geometry.prepared = PreparedExpansion::default();
    let settings = inputs.settings.as_deref().cloned().unwrap_or_default();
    if !settings.enabled || !geometry.supported {
        if !geometry.pages.is_empty() || geometry.residency.pending_len() > 0 {
            geometry.release_all();
        }
        return;
    }
    let geometry = &mut *geometry;
    geometry.residency.set_budget(
        settings
            .budget_bytes
            .min(geometry.memory_cap.unwrap_or(u64::MAX)),
    );
    let removed = sections.take_removed();
    let freed = !removed.is_empty();
    for removed in removed {
        geometry.residency.remove(removed);
    }
    for changed in sections.changed().collect::<Vec<_>>() {
        geometry.residency.enqueue(changed);
    }
    let camera = views
        .iter()
        .find(|(.., camera)| *camera)
        .map(|(_, view, ..)| view.world_from_view.translation().as_dvec3());
    // Sections waiting for room get another chance once room is freed or the camera has moved
    // enough that some of them may now be nearer than resident ones.
    let moved = camera.is_some_and(|camera| {
        geometry
            .retry_eye
            .is_none_or(|last| last.distance(camera) > RETRY_DISTANCE)
    });
    if freed || moved {
        geometry.residency.retry_waiting();
        geometry.retry_eye = camera.or(geometry.retry_eye);
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
    let rebased = camera.is_some_and(|camera| geometry.residency.follow_camera(camera));
    if rebased || geometry.inputs.as_ref() != Some(&expansion_inputs) {
        let resident: Vec<_> = sections.entities().collect();
        geometry.residency.enqueue_all(resident);
        geometry.inputs = Some(expansion_inputs);
    }
    let mut expanded = 0;
    if kernels_ready && let Some(origin) = geometry.residency.origin {
        let budget = if rebased {
            u32::MAX
        } else {
            FRAME_EXPANSION_FACES
        };
        expanded = plan_frame(
            geometry, &inputs, &buffers, &sections, textures, atlas, origin, &settings, budget,
            camera,
        );
    }
    geometry.complete_radius = camera.and_then(|eye| {
        geometry
            .residency
            .pending()
            .filter_map(|entity| sections.get(entity))
            .map(|section| section_distance(section.origin, eye))
            .min_by(f64::total_cmp)
            .map(|nearest| (nearest.sqrt() - SECTION_RADIUS).max(0.0) as f32)
    });
    geometry
        .log
        .observe(&geometry.residency, expanded, geometry.complete_radius);
    if geometry.residency.pending_len() == 0 {
        geometry.budget_horizon = None;
    }
    let far = inputs
        .frame
        .as_deref()
        .map(|frame| frame.render_distance)
        .filter(|far| *far > 0.0);
    build_draws(geometry, &inputs.device, &sources, &views, far);
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
    eye: Option<DVec3>,
) -> u32 {
    let tables = MaterialTables {
        materials: textures.assets.materials(),
        model_templates: textures.assets.model_templates(),
        model_quads: textures.assets.model_quads(),
    };
    let device = &*inputs.device;
    // Jobs per (kind, page), with face indices relative to the page.
    let mut jobs: HashMap<(usize, usize), Vec<[u32; 4]>> = HashMap::new();
    let mut planned_faces = 0_u32;
    let mut overflow = false;
    // Nearest first, so the budget and the frame's expansion go to what the camera sees most.
    let mut queue: Vec<(f64, Entity)> = geometry
        .residency
        .take_queue()
        .into_iter()
        .filter_map(|entity| {
            let section = sections.get(entity)?;
            Some((
                eye.map_or(0.0, |eye| section_distance(section.origin, eye)),
                entity,
            ))
        })
        .collect();
    queue.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut eviction = Eviction::default();
    let mut queue = queue.into_iter();
    while let Some((distance, entity)) = queue.next() {
        if planned_faces >= budget {
            geometry.residency.enqueue(entity);
            queue
                .by_ref()
                .for_each(|(_, rest)| geometry.residency.enqueue(rest));
            break;
        }
        let Some(section) = sections.get(entity) else {
            continue;
        };
        let mut plan: SectionPlan = plan_section(&section.geometry, &tables);
        if plan.faces == 0 {
            geometry.residency.remove(entity);
            continue;
        }
        let placement = geometry.residency.place_nearest(
            entity,
            &plan,
            eye.map(|eye| (eye, distance)),
            &mut eviction,
        );
        let Some(placement) = placement else {
            // Nothing farther is left to give up its room: this and every farther section
            // wait until room frees up or the camera moves.
            overflow = true;
            geometry.budget_horizon =
                eye.map(|_| (distance.sqrt() - SECTION_RADIUS).max(0.0) as f32);
            geometry.residency.wait(entity);
            queue
                .by_ref()
                .for_each(|(_, rest)| geometry.residency.wait(rest));
            break;
        };
        if !geometry.grow_page(device, &placement) {
            // Out of device memory: keep the pages there are, and let nearer sections take
            // room from farther ones from now on.
            geometry.residency.revoke(entity, &placement);
            let cap = geometry.residency.capacity_faces() * FACE_BYTES;
            geometry.memory_cap = Some(cap);
            geometry
                .residency
                .set_budget(settings.budget_bytes.min(cap));
            queue
                .by_ref()
                .for_each(|(_, rest)| geometry.residency.enqueue(rest));
            break;
        }
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
    if overflow && !geometry.overflow_reported {
        warn!(
            "shader-pack terrain exceeds its {} MiB vertex budget; distant sections wait undrawn",
            settings.budget_bytes >> 20
        );
    }
    geometry.overflow_reported |= overflow;
    if jobs.is_empty() {
        return planned_faces;
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
    planned_faces
}

/// Draw lists per camera view of its visible resident sections within `far` horizontally, as
/// Java culls render sections by render distance, plus the shadow list of every resident one.
fn build_draws(
    geometry: &mut ShaderpackGeometry,
    device: &RenderDevice,
    sources: &PackedChunkSources,
    views: &GeometryViews,
    far: Option<f32>,
) {
    let epoch = geometry.residency.epoch;
    let mut max_faces = 0;
    geometry.views.clear();
    let reach = far.map(|far| f64::from(far) + 8.0);
    for (entity, view, visible, _) in views.iter() {
        let eye = view.world_from_view.translation().as_dvec3();
        let within = |faces: &ResidentFaces| {
            reach.is_none_or(|reach| {
                let [x, _, z] = faces.origin;
                let dx = f64::from(x) + 8.0 - eye.x;
                let dz = f64::from(z) + 8.0 - eye.z;
                dx * dx + dz * dz <= reach * reach
            })
        };
        let sections = visible
            .get::<ChunkRenderInstance>()
            .into_iter()
            .flat_map(|class| class.iter_visible())
            .map(|(entity, _)| *entity)
            .filter(|entity| !sources.is_hidden(*entity))
            .filter_map(|entity| geometry.residency.sections.get(&entity))
            .filter(|faces| within(faces));
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
    timestamps: Option<Res<crate::gpu_timing::GpuTimestamps>>,
    mut context: RenderContext,
) {
    let prepared = std::mem::take(&mut geometry.prepared);
    if prepared.copies.is_empty() && prepared.dispatches.is_empty() {
        return;
    }
    crate::gpu_timing::within_stage(
        timestamps.as_deref(),
        RuntimeStage::GpuShaderpackExpansion,
        context.command_encoder(),
        |encoder| {
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
        },
    );
}

/// Once-a-second residency report, and the moment the expansion queue drains, as evidence of
/// how long terrain takes to fill in.
#[derive(Default)]
struct ResidencyLog {
    window_start: Option<Instant>,
    expanded_faces: u64,
    frames: u32,
    filling_since: Option<Instant>,
}

impl ResidencyLog {
    fn observe(&mut self, residency: &Residency, expanded: u32, complete_radius: Option<f32>) {
        let now = Instant::now();
        self.expanded_faces += u64::from(expanded);
        self.frames += 1;
        let pending = residency.pending_len();
        match (pending, self.filling_since) {
            (0, Some(since)) => {
                self.filling_since = None;
                debug!(
                    target: "cinnabar::shaderpack",
                    seconds = now.duration_since(since).as_secs_f32(),
                    sections = residency.sections.len(),
                    "shader-pack terrain complete"
                );
            }
            (1.., None) => self.filling_since = Some(now),
            _ => {}
        }
        let start = *self.window_start.get_or_insert(now);
        if now.duration_since(start) < Duration::from_secs(1) {
            return;
        }
        let resident_faces: u64 = residency
            .sections
            .values()
            .map(|section| u64::from(section.faces.end - section.faces.start))
            .sum();
        debug!(
            target: "cinnabar::shaderpack",
            sections = residency.sections.len(),
            resident_mib = (resident_faces * FACE_BYTES) >> 20,
            allocated_mib = (residency.capacity_faces() * FACE_BYTES) >> 20,
            pending,
            waiting = residency.waiting_len(),
            complete_radius,
            expanded_faces = self.expanded_faces,
            frames = self.frames,
            "shader-pack terrain residency"
        );
        *self = Self {
            filling_since: self.filling_since,
            ..Self::default()
        };
    }
}
