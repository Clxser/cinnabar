//! Rebuilds derived terrain state after an allocation failure, keeping allocated storage.

use super::*;
use crate::queue_writes::ProfiledQueue as _;
use bevy::ecs::change_detection::Mut;
use render_scene::ChunkRenderInstance;

#[derive(Resource)]
pub(in crate::chunk) struct TerrainGpuRecovery {
    failures: crate::gpu_allocation::FailureFlag,
    /// Published instances have already spent their linear publication permits.
    pub(in crate::chunk) published: HashMap<Entity, u64>,
    /// Keeps growth minimal until an entire terrain frame has succeeded.
    pub(in crate::chunk) recovering: bool,
}

impl TerrainGpuRecovery {
    /// Observes failures from all terrain owners, including allocation on worker threads.
    pub(in crate::chunk) fn new(device: &RenderDevice) -> Self {
        Self {
            failures: crate::gpu_allocation::FailureFlag::new(device),
            published: HashMap::new(),
            recovering: false,
        }
    }
}

/// Forgets every derived publication together; CPU instances remain the source of truth.
pub(in crate::chunk) fn begin_recovery(world: &mut World) {
    let failed = world.resource::<TerrainGpuRecovery>().failures.take();
    if !failed && world.resource::<TerrainGpuRecovery>().published.is_empty() {
        return;
    }
    let profiler = world
        .get_resource::<render_scene::RuntimeStageProfiler>()
        .cloned();
    let _timer = profiler
        .as_ref()
        .map(|profiler| profiler.time(render_scene::RuntimeStage::GpuPreparation));
    if !failed {
        world.resource_scope(|world, mut recovery: Mut<TerrainGpuRecovery>| {
            recovery.published.retain(|&entity, generation| {
                world
                    .get::<ChunkRenderInstance>(entity)
                    .is_some_and(|instance| instance.generation == *generation)
            });
        });
        return;
    }
    let published = world
        .resource::<ChunkGpuArena>()
        .allocations
        .iter()
        .map(|(&entity, allocation)| (entity, allocation.generation))
        .collect::<Vec<_>>();
    {
        let mut recovery = world.resource_mut::<TerrainGpuRecovery>();
        recovery.recovering = true;
        recovery.published.extend(published);
    }
    world.resource_scope(|world, mut recovery: Mut<TerrainGpuRecovery>| {
        recovery.published.retain(|&entity, generation| {
            world
                .get::<ChunkRenderInstance>(entity)
                .is_some_and(|instance| instance.generation == *generation)
        });
    });
    let entities = world
        .query_filtered::<Entity, With<GpuChunkAllocation>>()
        .iter(world)
        .collect::<Vec<_>>();
    for entity in entities {
        world.entity_mut(entity).remove::<GpuChunkAllocation>();
    }
    world
        .resource_mut::<ChunkGpuArena>()
        .invalidate_publication();
    *world.resource_mut::<GpuUpdateFairness>() = GpuUpdateFairness::default();
    *world.resource_mut::<coverage::TerrainCoverage>() = coverage::TerrainCoverage::default();
    {
        let mut retained = world.resource_mut::<pipeline::batches::RetainedTerrainBatches>();
        let stats = retained.stats;
        *retained = default();
        retained.stats = stats;
    }
    *world.resource_mut::<pipeline::solid::ChunkSolidIndirectBatches>() = default();
    *world.resource_mut::<ChunkIndirectBatches>() = default();
    *world.resource_mut::<ChunkModelIndirectBatches>() = default();
    *world.resource_mut::<ChunkDepthLiquidIndirectBatches>() = default();
    world
        .resource_mut::<TransparentSortRuntime>()
        .reset_for_view(None);
    let next_generation = world
        .resource::<TransparentModelSortRuntime>()
        .next_generation;
    *world.resource_mut::<TransparentModelSortRuntime>() = default();
    world
        .resource_mut::<TransparentModelSortRuntime>()
        .next_generation = next_generation;
    *world.resource_mut::<TransparentRetirementFence>() = default();
    *world.resource_mut::<TransparentPresentationFence>() = default();
    if let Some(mut units) = world.get_resource_mut::<pipeline::units::TerrainUnitBatches>() {
        units.invalidate_publication();
    }
    gpu_cull::invalidate_publication(world);
    if let Some(mut far) = world.get_resource_mut::<crate::far_terrain::FarTerrainGpu>() {
        far.invalidate_publication();
    }
    #[cfg(feature = "shaderpacks")]
    if let Some(mut geometry) =
        world.get_resource_mut::<crate::shaderpack::geometry::ShaderpackGeometry>()
    {
        geometry.invalidate_publication();
    }
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    crate::gpu_allocation::terrain_checked(
        device,
        "terrain recovery biome upload",
        std::mem::size_of_val(&FALLBACK_BIOME_RECORD) as u64,
        || {
            queue.profiled_write_buffer(
                &world.resource::<ChunkGpuArena>().biome_buffer,
                0,
                bytemuck::cast_slice(&FALLBACK_BIOME_RECORD),
            )
        },
    );
}

/// Prevents a partially prepared frame from drawing or witnessing terrain. Recovery retries
/// the entire publication next frame, regardless of which owner observed the failure.
pub(in crate::chunk) fn finish_publication(world: &mut World) {
    let mut recovery = world.resource_mut::<TerrainGpuRecovery>();
    if !recovery.failures.failed() && recovery.published.is_empty() {
        recovery.recovering = false;
        return;
    }
    let mut arena = world.resource_mut::<ChunkGpuArena>();
    arena.bind_group = None;
    arena.transparent_bind_group = None;
    gpu_cull::suppress_publication(world);
    crate::far_terrain::suppress_publication(world);
    #[cfg(feature = "shaderpacks")]
    if let Some(mut geometry) =
        world.get_resource_mut::<crate::shaderpack::geometry::ShaderpackGeometry>()
    {
        geometry.invalidate_publication();
    }
}

impl ChunkGpuArena {
    /// Reuses physical buffers after all previous frame commands have been submitted.
    fn invalidate_publication(&mut self) {
        self.allocations.clear();
        self.transparent_liquids = default();
        self.retired_allocations.clear();
        self.retirement_budget.items = 0;
        self.retirement_budget.bytes = 0;
        self.migration = None;
        self.quad_len = 0;
        self.geometry_stream_len = 0;
        self.origin_len = 0;
        self.biome_len = FALLBACK_BIOME_WORDS;
        self.free_quads.clear();
        self.free_geometry_stream_words.clear();
        self.free_origins.clear();
        self.free_biomes.clear();
        self.shared_biome_ranges = default();
        self.uploaded_indirect_bytes.clear();
        self.bind_group = None;
        self.transparent_bind_group = None;
        self.bind_group_buffers = None;
    }
}
