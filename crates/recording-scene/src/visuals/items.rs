//! Native item slots, source geometry and explicit rope inputs for independent replay.

use crate::binary::{self, Reader};
use recording_studio::{
    archive::{ArchiveError, MAX_RECORD_BYTES},
    object_store::{AssetId, AssetReader, AssetWriter},
};
use render_model::dropped_item::{
    ItemTerrainSnapshot, MAX_ITEM_TERRAIN_PUBLICATIONS, PREPARED_ITEM_ROPE_WORDS, PreparedItemRope,
    WHITE_LAYER,
};
use render_model::{
    DroppedItemInstance, DroppedItemModel, EquipmentRaster, ItemMeshVertex,
    MAX_DROPPED_ITEM_INSTANCES, MAX_DYNAMIC_ITEM_VERTICES, TerrainItemInstance,
    TerrainItemTransition,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

mod models;
pub(super) use models::Cache;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemCoverage {
    pub source_seconds: f64,
    pub model_failures: u32,
    pub instance_limit: bool,
    pub rope_limit: bool,
    pub terrain_pending: bool,
}

#[derive(Clone, Default)]
pub struct PreparedItems {
    pub models: Arc<[DroppedItemModel]>,
    pub instances: Arc<[DroppedItemInstance]>,
    pub terrain_session_id: Option<u64>,
    pub terrain_instances: Arc<[TerrainItemInstance]>,
    pub dynamic: Arc<[ItemMeshVertex]>,
    pub ropes: Arc<[PreparedItemRope]>,
    pub terrain: Option<ItemTerrainSnapshot>,
    pub daylight: f32,
    pub glint: Option<EquipmentRaster>,
    pub glint_scroll: [f32; 3],
    pub coverage: ItemCoverage,
}

impl PreparedItems {
    /// Static model templates and rasters share one immutable catalog allocation across samples.
    pub(super) fn model_retained_bytes(&self) -> usize {
        models::retained_bytes(&self.models)
    }

    /// Charges all shared model epochs and transition arrays before the capture queue retains them.
    pub fn retained_bytes(&self) -> usize {
        self.model_retained_bytes()
            + std::mem::size_of_val(&*self.instances)
            + std::mem::size_of_val(&*self.terrain_instances)
            + self
                .terrain_instances
                .iter()
                .map(|v| std::mem::size_of_val(&*v.transitions))
                .sum::<usize>()
            + std::mem::size_of_val(&*self.dynamic)
            + std::mem::size_of_val(&*self.ropes)
            + self
                .terrain
                .as_ref()
                .map_or(0, |t| std::mem::size_of_val(&*t.generations))
            + self.glint.as_ref().map_or(0, |v| v.rgba8.len())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Metadata {
    models: AssetId,
    terrain_session_id: Option<u64>,
    daylight: f32,
    glint: Option<Raster>,
    glint_scroll: [f32; 3],
    coverage: ItemCoverage,
    terrain: Option<(u64, Option<u64>, bool)>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raster {
    width: u16,
    height: u16,
    pixels: AssetId,
}

/// Model epochs are separate hash-checked objects; only bounded dynamic streams enter each sample.
pub(super) fn store(
    store: &mut AssetWriter,
    cache: &mut Cache,
    frame: &PreparedItems,
) -> Result<(Metadata, Vec<u8>), ArchiveError> {
    let metadata = Metadata {
        models: models::store(store, cache, &frame.models)?,
        terrain_session_id: frame.terrain_session_id,
        daylight: frame.daylight,
        glint: frame
            .glint
            .as_ref()
            .map(|r| {
                Ok::<_, ArchiveError>(Raster {
                    width: r.width,
                    height: r.height,
                    pixels: store.put_shared_pixels(&r.rgba8)?,
                })
            })
            .transpose()?,
        glint_scroll: frame.glint_scroll,
        coverage: frame.coverage,
        terrain: frame
            .terrain
            .as_ref()
            .map(|t| (t.serial, t.session_id, t.complete)),
    };
    let mut bytes = Vec::new();
    binary::packed(&mut bytes, &frame.instances, instance_words)?;
    binary::u32(&mut bytes, frame.terrain_instances.len() as u32)?;
    for candidate in frame.terrain_instances.iter() {
        binary::packed(&mut bytes, &[candidate.instance], instance_words)?;
        binary::u32(&mut bytes, u32::from(candidate.visible))?;
        binary::packed(&mut bytes, &candidate.transitions, |t| {
            [
                t.key.dimension as u32,
                t.key.x as u32,
                t.key.y as u32,
                t.key.z as u32,
                t.generation as u32,
                (t.generation >> 32) as u32,
                u32::from(t.visible),
            ]
        })?;
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(ArchiveError::Quota);
        }
    }
    binary::packed(&mut bytes, &frame.dynamic, |v| {
        bytemuck::cast::<_, [u32; std::mem::size_of::<ItemMeshVertex>() / 4]>(*v)
    })?;
    binary::packed(&mut bytes, &frame.ropes, |v| v.words())?;
    binary::packed(
        &mut bytes,
        frame.terrain.as_ref().map_or(&[], |t| &*t.generations),
        |(key, generation)| {
            [
                key.dimension as u32,
                key.x as u32,
                key.y as u32,
                key.z as u32,
                *generation as u32,
                (*generation >> 32) as u32,
            ]
        },
    )?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(ArchiveError::Quota);
    }
    Ok((metadata, bytes))
}

#[derive(Default)]
pub(super) struct Loader {
    models: Option<(AssetId, Arc<[DroppedItemModel]>)>,
}

/// Every texture, model reference, boolean and dynamic vertex is admitted before publication.
pub(super) fn load(
    loader: &mut Loader,
    store: &AssetReader,
    metadata: Metadata,
    bytes: &[u8],
    mut pixels: impl FnMut(AssetId, usize) -> Result<Arc<[u8]>, ArchiveError>,
    pixel_budget: &mut usize,
    cancelled: &impl Fn() -> bool,
) -> Result<PreparedItems, ArchiveError> {
    let Metadata {
        models: id,
        terrain_session_id,
        daylight,
        glint,
        glint_scroll,
        coverage,
        terrain,
    } = metadata;
    if !daylight.is_finite()
        || !(0.0..=1.0).contains(&daylight)
        || !glint_scroll.iter().all(|v| v.is_finite())
        || !coverage.source_seconds.is_finite()
        || coverage.source_seconds < 0.0
    {
        return Err(ArchiveError::Invalid(
            "invalid native item frame parameters",
        ));
    }
    let models = match &loader.models {
        Some((previous, models)) if *previous == id => {
            *pixel_budget = pixel_budget
                .checked_sub(models::pixel_bytes(models))
                .ok_or(ArchiveError::Quota)?;
            Arc::clone(models)
        }
        _ => {
            let models = models::load(store, id, &mut pixels, pixel_budget, cancelled)?;
            loader.models = Some((id, Arc::clone(&models)));
            models
        }
    };
    let mut input = bytes;
    let mut r = Reader::new(&mut input);
    let instances = r.packed_limit(MAX_DROPPED_ITEM_INSTANCES, |words| instance(words, &models))?;
    let count = r.u32()? as usize;
    if count > MAX_DROPPED_ITEM_INSTANCES {
        return Err(ArchiveError::Quota);
    }
    let mut terrain_instances = Vec::with_capacity(count);
    for _ in 0..count {
        if cancelled() {
            return Err(ArchiveError::Cancelled);
        }
        let mut item = r.packed_limit(1, |words| instance(words, &models))?;
        let instance = item
            .pop()
            .ok_or(ArchiveError::Invalid("missing terrain item instance"))?;
        let visible = boolean(r.u32()?)?;
        let transitions = r.packed_limit(
            MAX_RECORD_BYTES / std::mem::size_of::<TerrainItemTransition>(),
            |w: [u32; 7]| {
                let key =
                    world::SubChunkKey::new(w[0] as i32, w[1] as i32, w[2] as i32, w[3] as i32);
                crate::scene::ChunkId::from(key).origin().ok()?;
                Some(TerrainItemTransition {
                    key,
                    generation: u64::from(w[4]) | (u64::from(w[5]) << 32),
                    visible: boolean(w[6]).ok()?,
                })
            },
        )?;
        terrain_instances.push(TerrainItemInstance {
            instance,
            visible,
            transitions: transitions.into(),
        });
    }
    let dynamic = r.packed_limit(
        MAX_DYNAMIC_ITEM_VERTICES,
        |w: [u32; std::mem::size_of::<ItemMeshVertex>() / 4]| {
            let v: ItemMeshVertex = bytemuck::cast(w);
            (v.layer == WHITE_LAYER
                && v.position
                    .iter()
                    .chain(&v.uv)
                    .chain(&v.normal)
                    .chain(&v.glint_scale)
                    .all(|v| v.is_finite()))
            .then_some(v)
        },
    )?;
    if dynamic.len() % 3 != 0 {
        return Err(ArchiveError::Invalid("incomplete recorded item triangle"));
    }
    let ropes = r.packed_limit(
        MAX_DYNAMIC_ITEM_VERTICES / 6,
        |w: [u32; PREPARED_ITEM_ROPE_WORDS]| PreparedItemRope::from_words(w),
    )?;
    if ropes.iter().map(|r| r.segments as usize * 6).sum::<usize>() > 2 * MAX_DYNAMIC_ITEM_VERTICES
    {
        return Err(ArchiveError::Quota);
    }
    let generations = r.packed_limit(MAX_ITEM_TERRAIN_PUBLICATIONS, |w: [u32; 6]| {
        let key = world::SubChunkKey::new(w[0] as i32, w[1] as i32, w[2] as i32, w[3] as i32);
        crate::scene::ChunkId::from(key).origin().ok()?;
        Some((key, u64::from(w[4]) | (u64::from(w[5]) << 32)))
    })?;
    if generations.windows(2).any(|w| w[0].0 >= w[1].0)
        || (terrain.is_none() && !generations.is_empty())
    {
        return Err(ArchiveError::Invalid(
            "invalid recorded item terrain generations",
        ));
    }
    let terrain = terrain.map(|(serial, session_id, complete)| ItemTerrainSnapshot {
        serial,
        session_id,
        complete,
        generations: generations.into(),
    });
    if terrain
        .as_ref()
        .is_some_and(|t| t.session_id != terrain_session_id)
    {
        return Err(ArchiveError::Invalid(
            "item terrain publication belongs to another session",
        ));
    }
    if coverage.terrain_pending != terrain.as_ref().is_none_or(|t| !t.complete) {
        return Err(ArchiveError::Invalid("inconsistent item terrain coverage"));
    }
    drop(r);
    binary::finish(&mut input)?;
    let glint = glint
        .map(|r| {
            let size = super::pixel_bytes(u32::from(r.width), u32::from(r.height))?;
            *pixel_budget = pixel_budget.checked_sub(size).ok_or(ArchiveError::Quota)?;
            Ok::<_, ArchiveError>(EquipmentRaster {
                width: r.width,
                height: r.height,
                rgba8: pixels(r.pixels, size)?,
            })
        })
        .transpose()?;
    Ok(PreparedItems {
        models,
        instances: instances.into(),
        terrain_session_id,
        terrain_instances: terrain_instances.into(),
        dynamic: dynamic.into(),
        ropes: ropes.into(),
        terrain,
        daylight,
        glint,
        glint_scroll,
        coverage,
    })
}

/// Instance booleans use explicit words instead of Rust layout or unvalidated enum bytes.
fn instance_words(v: &DroppedItemInstance) -> [u32; 17] {
    let mut w = [0; 17];
    w[0] = v.model;
    for (word, value) in w[1..13].iter_mut().zip(v.world_from_item.iter().flatten()) {
        *word = value.to_bits();
    }
    w[13] = v.block_level;
    w[14] = v.sky_level;
    w[15] = v.overlay_rgba8;
    w[16] = u32::from(v.glint);
    w
}

/// Valid item references always address a real admitted model and finite native transform.
fn instance(w: [u32; 17], models: &[DroppedItemModel]) -> Option<DroppedItemInstance> {
    if matches!(
        models.get(w[0] as usize),
        None | Some(DroppedItemModel::Vacant)
    ) || w[13] > 15
        || w[14] > 15
    {
        return None;
    }
    let mut matrix = [[0.0; 4]; 3];
    for (value, word) in matrix.iter_mut().flatten().zip(&w[1..13]) {
        *value = f32::from_bits(*word);
        if !value.is_finite() {
            return None;
        }
    }
    Some(DroppedItemInstance {
        model: w[0],
        world_from_item: matrix,
        block_level: w[13],
        sky_level: w[14],
        overlay_rgba8: w[15],
        glint: boolean(w[16]).ok()?,
    })
}

/// Invalid selectors never become true through a nonzero cast.
fn boolean(word: u32) -> Result<bool, ArchiveError> {
    match word {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(ArchiveError::Invalid("invalid recorded item boolean")),
    }
}
