//! Independently owned terrain scenes. Seeking never mutates an authoritative live world.

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Weak},
};

use recording_studio::{
    archive::{ArchiveError, MAX_RECORD_BYTES},
    object_store::{AssetId, AssetReader, MAX_ASSET_BYTES, MAX_RECORDING_ASSETS},
};
use serde::{Deserialize, Serialize};

use crate::{
    palette::{self, FrozenPalette, FrozenTint, PaletteBinding},
    terrain::TerrainMesh,
};

pub const MAX_SCENE_CHUNKS: usize = 65_536;
pub const DEFAULT_RESIDENT_BYTES: usize = 2 * MAX_ASSET_BYTES as usize;
pub(crate) const MAX_LINK_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChunkId {
    pub dimension: i32,
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl From<world::SubChunkKey> for ChunkId {
    fn from(key: world::SubChunkKey) -> Self {
        let world::SubChunkKey { dimension, x, y, z } = key;
        Self { dimension, x, y, z }
    }
}

impl ChunkId {
    /// Mesh origins use checked block coordinates, rejecting overflow before render conversion.
    pub fn origin(self) -> Result<[i32; 3], ArchiveError> {
        let mut origin = [0; 3];
        for (destination, source) in origin.iter_mut().zip([self.x, self.y, self.z]) {
            *destination =
                source
                    .checked_mul(world::SUB_CHUNK_SIDE as i32)
                    .ok_or(ArchiveError::Invalid(
                        "recorded terrain origin overflows block coordinates",
                    ))?;
        }
        Ok(origin)
    }

    /// Reconstructs the owner key only after its render origin has been admitted.
    pub fn key(self) -> Result<world::SubChunkKey, ArchiveError> {
        self.origin()?;
        Ok(world::SubChunkKey::new(
            self.dimension,
            self.x,
            self.y,
            self.z,
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChunkBinding {
    pub key: ChunkId,
    pub mesh: AssetId,
    pub palette: AssetId,
    pub generation: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "change", deny_unknown_fields)]
pub enum TerrainRecord {
    SelectPalette { palette: AssetId },
    Upsert { chunk: ChunkBinding },
    Remove { key: ChunkId },
}

#[derive(Clone)]
pub struct ScenePalette {
    pub id: AssetId,
    pub palette: Arc<FrozenPalette>,
}

#[derive(Clone)]
pub struct SceneChunk {
    pub binding: ChunkBinding,
    pub mesh: Arc<TerrainMesh>,
    pub palette: Arc<FrozenPalette>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ResourceKey {
    World(AssetId),
    Tints(AssetId),
    Palette(AssetId),
    Mesh(AssetId, AssetId),
}

#[derive(Clone, Copy)]
struct Charge {
    bytes: usize,
    references: usize,
}

pub struct TerrainScene {
    chunks: BTreeMap<ChunkId, SceneChunk>,
    active_palette: Option<ScenePalette>,
    resources: BTreeMap<ResourceKey, Charge>,
    resident_bytes: usize,
    maximum_bytes: usize,
}

impl TerrainScene {
    /// A staging scene has explicit chunk and unique allocation budgets independent of live settings.
    pub fn new(maximum_bytes: usize) -> Self {
        Self {
            chunks: BTreeMap::new(),
            active_palette: None,
            resources: BTreeMap::new(),
            resident_bytes: 0,
            maximum_bytes,
        }
    }

    pub fn chunks(&self) -> impl ExactSizeIterator<Item = &SceneChunk> {
        self.chunks.values()
    }
    /// Looks up an already admitted binding without scanning the recorded world.
    pub fn chunk(&self, key: ChunkId) -> Option<&SceneChunk> {
        self.chunks.get(&key)
    }

    /// The selected render palette is explicit even when no chunks are resident.
    pub fn active_palette(&self) -> Option<&ScenePalette> {
        self.active_palette.as_ref()
    }

    /// Retains the selected texture and tint epoch independently of any chunk using it.
    pub fn select_palette(&mut self, palette: ScenePalette) -> Result<(), ArchiveError> {
        if self
            .active_palette
            .as_ref()
            .is_some_and(|old| old.id == palette.id)
        {
            return Ok(());
        }
        let mut changes = BTreeMap::new();
        if let Some(old) = &self.active_palette {
            for (key, bytes) in palette_allocations(old.id, &old.palette) {
                changes.insert(key, (bytes, -1));
            }
        }
        add_allocations(
            &mut changes,
            palette_allocations(palette.id, &palette.palette),
        )?;
        self.apply_allocations(changes, 0)?;
        self.active_palette = Some(palette);
        Ok(())
    }
    pub const fn resident_bytes(&self) -> usize {
        self.resident_bytes
    }

    /// Clears only this draft; a renderer keeps the last admitted scene until a whole seek succeeds.
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.active_palette = None;
        self.resources.clear();
        self.resident_bytes = 0;
    }

    /// Replaces a chunk atomically after charging shared world, tint and mesh buffers only once.
    /// Admission touches at most eight resource keys, without rescanning or cloning a growing world.
    pub fn upsert(&mut self, chunk: SceneChunk) -> Result<(), ArchiveError> {
        chunk.binding.key.origin()?;
        if self.chunks.len() >= MAX_SCENE_CHUNKS && !self.chunks.contains_key(&chunk.binding.key) {
            return Err(ArchiveError::Invalid(
                "recorded scene exceeds its chunk limit",
            ));
        }
        let mut changes = BTreeMap::<ResourceKey, (usize, isize)>::new();
        if let Some(old) = self.chunks.get(&chunk.binding.key) {
            for (key, bytes) in allocations(old) {
                changes.insert(key, (bytes, -1));
            }
        }
        add_allocations(&mut changes, allocations(&chunk))?;
        let new_chunk = !self.chunks.contains_key(&chunk.binding.key);
        self.apply_allocations(changes, if new_chunk { chunk_charge() } else { 0 })?;
        self.chunks.insert(chunk.binding.key, chunk);
        Ok(())
    }

    /// Validates a complete reference-count transaction before changing any resident charges.
    fn apply_allocations(
        &mut self,
        changes: BTreeMap<ResourceKey, (usize, isize)>,
        additional_bytes: usize,
    ) -> Result<(), ArchiveError> {
        let mut resident = self.resident_bytes;
        for (key, (bytes, delta)) in &changes {
            let old = self.resources.get(key);
            if old.is_some_and(|value| value.bytes != *bytes) {
                return Err(ArchiveError::Invalid("inconsistent frozen scene resource"));
            }
            let references = old
                .map_or(0, |value| value.references)
                .checked_add_signed(*delta)
                .ok_or(ArchiveError::Invalid("recorded scene reference overflow"))?;
            if old.is_none() && references != 0 {
                resident = resident.checked_add(*bytes).ok_or(ArchiveError::Quota)?;
            } else if old.is_some() && references == 0 {
                resident = resident.checked_sub(*bytes).ok_or(ArchiveError::Quota)?;
            }
        }
        resident = resident
            .checked_add(additional_bytes)
            .ok_or(ArchiveError::Quota)?;
        if resident > self.maximum_bytes {
            return Err(ArchiveError::Quota);
        }
        for (key, (bytes, delta)) in changes {
            let references = self
                .resources
                .get(&key)
                .map_or(0, |value| value.references)
                .checked_add_signed(delta)
                .ok_or(ArchiveError::Invalid("recorded scene reference overflow"))?;
            if references == 0 {
                self.resources.remove(&key);
            } else {
                self.resources.insert(key, Charge { bytes, references });
            }
        }
        self.resident_bytes = resident;
        Ok(())
    }

    /// An unload drops the last reference to unused objects; unknown unloads are harmless.
    pub fn remove(&mut self, key: ChunkId) {
        let Some(chunk) = self.chunks.remove(&key) else {
            return;
        };
        self.resident_bytes -= chunk_charge();
        for (key, _) in allocations(&chunk) {
            if let Some(charge) = self.resources.get_mut(&key) {
                charge.references -= 1;
                if charge.references == 0 {
                    self.resident_bytes -= charge.bytes;
                    self.resources.remove(&key);
                }
            }
        }
    }
}

/// Includes map ownership overhead in addition to the separately charged immutable allocations.
fn chunk_charge() -> usize {
    std::mem::size_of::<SceneChunk>()
        + std::mem::size_of::<ChunkId>()
        + 8 * std::mem::size_of::<usize>()
}

/// Mesh admission depends on a palette, so independently admitted copies have independent charges.
fn allocations(chunk: &SceneChunk) -> [(ResourceKey, usize); 4] {
    let [world, tints, palette] = palette_allocations(chunk.binding.palette, &chunk.palette);
    [
        world,
        tints,
        palette,
        (
            ResourceKey::Mesh(chunk.binding.mesh, chunk.binding.palette),
            chunk.mesh.retained_bytes(),
        ),
    ]
}

/// Active selection and chunk bindings share the same immutable allocation identities.
fn palette_allocations(id: AssetId, palette: &FrozenPalette) -> [(ResourceKey, usize); 3] {
    [
        (
            ResourceKey::World(palette.binding.world),
            palette.world_bytes(),
        ),
        (
            ResourceKey::Tints(palette.binding.tints),
            std::mem::size_of_val(&*palette.tints),
        ),
        (
            ResourceKey::Palette(id),
            std::mem::size_of::<FrozenPalette>() + 8 * std::mem::size_of::<usize>(),
        ),
    ]
}

/// Merges incoming references without accepting conflicting sizes for one immutable identity.
fn add_allocations(
    changes: &mut BTreeMap<ResourceKey, (usize, isize)>,
    allocations: impl IntoIterator<Item = (ResourceKey, usize)>,
) -> Result<(), ArchiveError> {
    for (key, bytes) in allocations {
        let change = changes.entry(key).or_insert((bytes, 0));
        if change.0 != bytes {
            return Err(ArchiveError::Invalid(
                "inconsistent frozen scene allocation",
            ));
        }
        change.1 += 1;
    }
    Ok(())
}

/// Weak caches share immutable objects across checkpoints without keeping discarded scenes alive.
pub struct SceneLoader {
    store: AssetReader,
    worlds: BTreeMap<AssetId, Weak<assets::RuntimeAssets>>,
    tints: BTreeMap<AssetId, Weak<[FrozenTint]>>,
    palettes: BTreeMap<AssetId, Weak<FrozenPalette>>,
    meshes: BTreeMap<(AssetId, AssetId), Weak<TerrainMesh>>,
}

impl SceneLoader {
    /// The recording path is resolved by its project, never taken from object payloads.
    pub fn new(path: &Path) -> Self {
        Self {
            store: AssetReader::new(path),
            worlds: BTreeMap::new(),
            tints: BTreeMap::new(),
            palettes: BTreeMap::new(),
            meshes: BTreeMap::new(),
        }
    }

    /// Verifies the link, material epoch and mesh before exposing a renderable chunk to staging.
    pub fn load(&mut self, binding: ChunkBinding) -> Result<SceneChunk, ArchiveError> {
        self.load_cancellable(binding, || false)
    }

    /// Cancellation reaches large object reads as well as checkpoint traversal.
    pub fn load_cancellable(
        &mut self,
        binding: ChunkBinding,
        cancelled: impl Fn() -> bool,
    ) -> Result<SceneChunk, ArchiveError> {
        if cancelled() {
            return Err(ArchiveError::Cancelled);
        }
        binding.key.origin()?;
        let palette = self.palette(binding.palette, &cancelled)?;
        let cache_key = (binding.mesh, binding.palette);
        let mesh = match self.meshes.get(&cache_key).and_then(Weak::upgrade) {
            Some(mesh) => mesh,
            None => {
                let mesh = self.store.read_cancellable(
                    binding.mesh,
                    MAX_RECORD_BYTES as u64,
                    &cancelled,
                    |input| {
                        TerrainMesh::read(input, &palette.assets, palette.tints.len()).map(Arc::new)
                    },
                )?;
                prune(&mut self.meshes)?;
                self.meshes.insert(cache_key, Arc::downgrade(&mesh));
                mesh
            }
        };
        Ok(SceneChunk {
            binding,
            mesh,
            palette,
        })
    }

    /// Loads exact frozen references once per material epoch, retaining no live texture-table fallback.
    pub(crate) fn palette(
        &mut self,
        id: AssetId,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Arc<FrozenPalette>, ArchiveError> {
        if let Some(palette) = self.palettes.get(&id).and_then(Weak::upgrade) {
            return Ok(palette);
        }
        let binding: PaletteBinding =
            self.store
                .read_cancellable(id, MAX_LINK_BYTES as u64, cancelled, |input| {
                    Ok(serde_json::from_reader(input)?)
                })?;
        let assets = match self.worlds.get(&binding.world).and_then(Weak::upgrade) {
            Some(assets) => assets,
            None => {
                let assets =
                    palette::load_world_cancellable(&self.store, binding.world, cancelled)?;
                prune(&mut self.worlds)?;
                self.worlds.insert(binding.world, Arc::downgrade(&assets));
                assets
            }
        };
        let tints = match self.tints.get(&binding.tints).and_then(Weak::upgrade) {
            Some(tints) => tints,
            None => {
                let tints = palette::load_tints_cancellable(&self.store, binding.tints, cancelled)?;
                prune(&mut self.tints)?;
                self.tints.insert(binding.tints, Arc::downgrade(&tints));
                tints
            }
        };
        if tints.len() != binding.tint_count as usize {
            return Err(ArchiveError::Invalid(
                "recorded tint count does not match its table",
            ));
        }
        let palette = Arc::new(FrozenPalette::new(binding, assets, tints));
        prune(&mut self.palettes)?;
        self.palettes.insert(id, Arc::downgrade(&palette));
        Ok(palette)
    }
}

/// Discarded scenes leave weak entries only; bounded pruning never retains all visited recordings.
fn prune<K: Ord, T: ?Sized>(cache: &mut BTreeMap<K, Weak<T>>) -> Result<(), ArchiveError> {
    if cache.len() >= MAX_RECORDING_ASSETS {
        cache.retain(|_, value| value.strong_count() != 0);
    }
    if cache.len() >= MAX_RECORDING_ASSETS {
        return Err(ArchiveError::Quota);
    }
    Ok(())
}
