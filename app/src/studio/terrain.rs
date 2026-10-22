//! Bridges prepared native terrain into an independently owned recording track.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use bevy::prelude::*;
use recording_scene::{
    capture::{PreparedChunk, PreparedPalette, SceneChange, TerrainCapture},
    palette::FrozenTint,
    scene::{ChunkId, MAX_SCENE_CHUNKS},
    terrain::TerrainMesh,
};
use recording_studio::{MediaTime, capture::CaptureSession};
use render_scene::{
    BiomeTint, ChunkBiomeTints, ChunkRenderInstance, ChunkTextureAssetIdentity, ChunkTextureAssets,
    chunk::biome_tints::ChunkBiomeTintResourceIdentity,
};

use super::NativeCapture;

pub(super) struct NativeTerrain {
    pub capture: TerrainCapture,
    palette: Arc<PreparedPalette>,
    texture: ChunkTextureAssetIdentity,
    tint: ChunkBiomeTintResourceIdentity,
    entities: HashMap<Entity, ChunkId>,
    owners: BTreeMap<ChunkId, Entity>,
}

/// Takes one coherent baseline from immutable published meshes, with no remeshing or byte encoding.
pub(super) fn start(world: &mut World, session: &CaptureSession) -> Result<NativeTerrain, String> {
    let textures = world
        .get_resource::<ChunkTextureAssets>()
        .ok_or("World textures are unavailable for recording.")?
        .clone();
    let tints = world
        .get_resource::<ChunkBiomeTints>()
        .ok_or("World biome colours are unavailable for recording.")?
        .clone();
    let capture = TerrainCapture::start(session.directory().to_path_buf(), session.info().clone())?;
    let palette = prepare_palette(&capture, &textures, &tints)?;
    let mut terrain = NativeTerrain {
        capture,
        palette,
        texture: textures.identity(),
        tint: tints.resource_identity(),
        entities: HashMap::new(),
        owners: BTreeMap::new(),
    };
    let mut chunks = Vec::new();
    let mut query = world.query::<(Entity, &ChunkRenderInstance)>();
    for (entity, instance) in query.iter(world) {
        if instance.tint_identity != tints.identity {
            continue;
        }
        if chunks.len() >= MAX_SCENE_CHUNKS {
            return Err("The world exceeds the recording scene's chunk budget.".into());
        }
        chunks.push(terrain.upsert(entity, instance)?);
    }
    terrain
        .capture
        .baseline(Arc::clone(&terrain.palette), chunks)?;
    Ok(terrain)
}

/// Change filters reuse the live publication schedule. Idle recording does not scan the world.
/// Pack or tint replacement alone requires a full retag so every mesh keeps its exact asset epoch.
pub(super) fn observe(
    mut state: ResMut<NativeCapture>,
    clock: Res<Time<Real>>,
    textures: Option<Res<ChunkTextureAssets>>,
    tints: Option<Res<ChunkBiomeTints>>,
    all: Query<(Entity, &ChunkRenderInstance)>,
    changed: Query<(Entity, &ChunkRenderInstance), Changed<ChunkRenderInstance>>,
    mut removed: RemovedComponents<ChunkRenderInstance>,
) {
    let Some(active) = state.active.as_mut() else {
        removed.clear();
        return;
    };
    if active.stopping.is_some()
        || active
            .native_failures
            .contains(super::failures::NativeTrack::Terrain)
    {
        removed.clear();
        return;
    }
    let Some(terrain) = active.terrain.as_mut() else {
        removed.clear();
        return;
    };
    let now = clock.elapsed_secs_f64();
    let result = (|| {
        let textures = textures
            .as_deref()
            .ok_or("The recorded world's texture table was removed.")?;
        let tints = tints
            .as_deref()
            .ok_or("The recorded world's biome table was removed.")?;
        let epoch_changed =
            terrain.texture != textures.identity() || terrain.tint != tints.resource_identity();
        if epoch_changed {
            terrain.palette = prepare_palette(&terrain.capture, textures, tints)?;
            terrain.texture = textures.identity();
            terrain.tint = tints.resource_identity();
        }
        let mut changes = Vec::new();
        for entity in removed.read() {
            if let Some(key) = terrain.entities.remove(&entity)
                && terrain.owners.get(&key) == Some(&entity)
            {
                terrain.owners.remove(&key);
                changes.push(SceneChange::Remove(key));
            }
        }
        let mut admit = |entity, instance: &ChunkRenderInstance| -> Result<(), String> {
            if instance.tint_identity != tints.identity {
                return Ok(());
            }
            if changes.len() >= MAX_SCENE_CHUNKS {
                return Err("Too many terrain changes arrived in one recording frame.".into());
            }
            changes.push(terrain.upsert(entity, instance)?);
            Ok(())
        };
        if epoch_changed {
            for (entity, instance) in &all {
                admit(entity, instance)?;
            }
        } else {
            for (entity, instance) in &changed {
                admit(entity, instance)?;
            }
        }
        let at = MediaTime::from_seconds((now - active.started).max(0.0))
            .map_err(|error| error.to_string())?;
        terrain
            .capture
            .changes(at, Arc::clone(&terrain.palette), changes)
    })();
    if let Err(error) = result {
        super::failures::interrupt(active, super::failures::NativeTrack::Terrain, error, now);
    }
}

impl NativeTerrain {
    /// Records ownership independently of ECS entity reuse, so an old removal cannot erase a replacement.
    fn upsert(
        &mut self,
        entity: Entity,
        instance: &ChunkRenderInstance,
    ) -> Result<SceneChange, String> {
        let key = ChunkId::from(instance.key);
        if self.owners.len() >= MAX_SCENE_CHUNKS && !self.owners.contains_key(&key) {
            return Err("Recorded terrain exceeds its resident chunk limit.".into());
        }
        key.origin().map_err(|error| error.to_string())?;
        if self
            .entities
            .get(&entity)
            .is_some_and(|previous| *previous != key)
        {
            return Err(
                "A recorded terrain entity changed its world key without an unload.".into(),
            );
        }
        if let Some(previous) = self.owners.insert(key, entity)
            && previous != entity
        {
            self.entities.remove(&previous);
        }
        self.entities.insert(entity, key);
        Ok(SceneChange::Upsert(PreparedChunk {
            key,
            generation: instance.generation,
            mesh: freeze_mesh(instance),
        }))
    }
}

/// Every prepared stream is immutable and shared. Publication permits and live scheduling stay native.
fn freeze_mesh(instance: &ChunkRenderInstance) -> TerrainMesh {
    let ChunkRenderInstance {
        key: _,
        cube_quads,
        cube_lighting,
        cube_layout,
        model_refs,
        model_lighting,
        model_draw_refs,
        transparent_model_draw_refs,
        liquid_quads,
        liquid_lighting,
        has_depth_liquid: _,
        has_transparent_liquid: _,
        depth_liquid_start: _,
        biome,
        tint_identity: _,
        generation: _,
        priority: _,
        token: _,
        publication_permit: _,
        origin: _,
    } = instance;
    TerrainMesh {
        cubes: Arc::clone(cube_quads),
        cube_lighting: Arc::clone(cube_lighting),
        cube_layout: *cube_layout,
        models: Arc::clone(model_refs),
        model_lighting: Arc::clone(model_lighting),
        model_draws: Arc::clone(model_draw_refs),
        transparent_model_draws: Arc::clone(transparent_model_draw_refs),
        liquids: Arc::clone(liquid_quads),
        liquid_lighting: Arc::clone(liquid_lighting),
        biome: biome.clone(),
    }
}

/// Exhaustive field binding keeps new native biome channels from being silently omitted in recordings.
fn freeze_tint(tint: &BiomeTint) -> FrozenTint {
    let &BiomeTint {
        grass,
        foliage,
        birch,
        evergreen,
        dry_foliage,
        seasonal_foliage,
        water,
        water_opacity,
        flags,
    } = tint;
    FrozenTint {
        grass,
        foliage,
        birch,
        evergreen,
        dry_foliage,
        seasonal_foliage,
        water,
        water_opacity,
        flags,
    }
}

/// Tint conversion happens once per actual resource identity; texture allocations are never copied here.
fn prepare_palette(
    capture: &TerrainCapture,
    textures: &ChunkTextureAssets,
    tints: &ChunkBiomeTints,
) -> Result<Arc<PreparedPalette>, String> {
    let entries = tints
        .entries
        .iter()
        .map(freeze_tint)
        .collect::<Vec<_>>()
        .into();
    capture.palette(
        Arc::clone(&textures.assets),
        entries,
        tints.identity.stream(),
        tints.identity.revision(),
    )
}
