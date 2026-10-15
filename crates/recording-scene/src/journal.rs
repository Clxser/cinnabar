//! Worker-owned scene publication and checkpoint restoration over the shared recording container.

use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    sync::{Arc, Weak},
};

use assets::RuntimeAssets;
use recording_studio::{
    MediaTime,
    archive::{
        ArchiveError, RecordInput, RecordKind, Recording, RecordingConfig, RecordingInfo,
        RecordingReader, RecordingTask,
    },
    object_store::{AssetId, AssetWriter},
};

use crate::{
    TERRAIN_TRACK_SCHEMA,
    palette::{self, FrozenTint, PaletteBinding},
    scene::{
        ChunkBinding, ChunkId, MAX_LINK_BYTES, MAX_SCENE_CHUNKS, SceneLoader, TerrainRecord,
        TerrainScene,
    },
    terrain::TerrainMesh,
};

pub const TERRAIN_DIRECTORY: &str = "terrain.cinreplay";
const IDENTITY_CACHE: usize = 8;

mod restore;
pub(crate) use restore::apply_record;

pub struct JournalPalette {
    id: AssetId,
    assets: Arc<RuntimeAssets>,
    tint_count: usize,
}

pub struct TerrainJournal {
    recording: Recording,
    objects: AssetWriter,
    chunks: BTreeMap<ChunkId, ChunkBinding>,
    checkpoint_at: Option<MediaTime>,
    worlds: VecDeque<(Weak<RuntimeAssets>, AssetId)>,
    tints: VecDeque<(Weak<[FrozenTint]>, AssetId)>,
}

impl TerrainJournal {
    /// Polls asynchronous compression and disk failures without waiting for another scene change.
    pub fn error(&self) -> Option<String> {
        self.recording.task().progress()?.error
    }

    /// Creates an optional terrain track beside POV media. Call only from a capture IO worker.
    /// Object and journal quotas are independent, explicit budgets supplied by the session owner.
    pub fn start(
        parent: &Path,
        mut info: RecordingInfo,
        config: RecordingConfig,
        object_quota: u64,
    ) -> Result<Self, ArchiveError> {
        info.scene_schema = TERRAIN_TRACK_SCHEMA.into();
        let root = parent.join(TERRAIN_DIRECTORY);
        let recording = Recording::start_ready(root.clone(), info, config)?;
        let objects = AssetWriter::open(&root, object_quota)?;
        Ok(Self {
            recording,
            objects,
            chunks: BTreeMap::new(),
            checkpoint_at: None,
            worlds: VecDeque::new(),
            tints: VecDeque::new(),
        })
    }

    /// Freezes each immutable world and tint allocation once while it remains in the recent cache.
    /// A weak identity cache avoids repeatedly serializing high-resolution textures or pinning old packs.
    pub fn palette(
        &mut self,
        assets: &Arc<RuntimeAssets>,
        tints: &Arc<[FrozenTint]>,
        stream: u64,
        revision: u64,
    ) -> Result<JournalPalette, ArchiveError> {
        let world = match cached(&self.worlds, assets) {
            Some(id) => id,
            None => {
                let id = palette::store_world(&mut self.objects, assets)?;
                remember(&mut self.worlds, assets, id);
                id
            }
        };
        let tint_id = match cached(&self.tints, tints) {
            Some(id) => id,
            None => {
                let id = palette::store_tints(&mut self.objects, tints)?;
                remember(&mut self.tints, tints, id);
                id
            }
        };
        let binding = PaletteBinding {
            world,
            tints: tint_id,
            tint_count: u32::try_from(tints.len())
                .map_err(|_| ArchiveError::Invalid("recorded tint count overflows"))?,
            tint_stream: stream,
            tint_revision: revision,
        };
        let bytes = encode(&binding)?;
        let id = self.objects.put(MAX_LINK_BYTES as u64, |output| {
            output
                .write_all(&bytes)
                .map_err(|_| ArchiveError::Invalid("cannot write recorded palette binding"))
        })?;
        Ok(JournalPalette {
            id,
            assets: Arc::clone(assets),
            tint_count: tints.len(),
        })
    }

    /// Streams prepared geometry and publishes its small reference only after durable object creation.
    pub fn upsert(
        &mut self,
        at: MediaTime,
        key: ChunkId,
        generation: u64,
        mesh: &TerrainMesh,
        palette: &JournalPalette,
    ) -> Result<(), ArchiveError> {
        key.origin()?;
        if self.chunks.len() >= MAX_SCENE_CHUNKS && !self.chunks.contains_key(&key) {
            return Err(ArchiveError::Quota);
        }
        if self
            .checkpoint_at
            .is_some_and(|checkpoint| checkpoint != at)
        {
            return Err(ArchiveError::Invalid(
                "recorded terrain checkpoint changed time",
            ));
        }
        mesh.validate(&palette.assets)?;
        if !mesh.biome.valid_for_recorded_tints(palette.tint_count) {
            return Err(ArchiveError::Invalid(
                "recorded mesh does not match its tint table",
            ));
        }
        let mesh = self.objects.put(
            recording_studio::archive::MAX_RECORD_BYTES as u64,
            |output| mesh.write(output),
        )?;
        let chunk = ChunkBinding {
            key,
            mesh,
            palette: palette.id,
            generation,
        };
        let kind = if self.checkpoint_at.is_some() {
            RecordKind::CheckpointPart
        } else {
            RecordKind::Scene
        };
        write_record(
            &mut self.recording,
            at,
            kind,
            &TerrainRecord::Upsert {
                chunk: chunk.clone(),
            },
        )?;
        self.chunks.insert(key, chunk);
        Ok(())
    }

    /// Removes only previously published terrain; checkpoint contents are complete upsert snapshots.
    pub fn remove(&mut self, at: MediaTime, key: ChunkId) -> Result<(), ArchiveError> {
        if self.checkpoint_at.is_some() {
            return Err(ArchiveError::Invalid(
                "cannot unload terrain inside a checkpoint",
            ));
        }
        if self.chunks.contains_key(&key) {
            write_record(
                &mut self.recording,
                at,
                RecordKind::Scene,
                &TerrainRecord::Remove { key },
            )?;
            self.chunks.remove(&key);
        }
        Ok(())
    }

    /// Starts the initial multipart baseline. No large scene clone or single giant payload is needed.
    pub fn begin_baseline(&mut self) -> Result<(), ArchiveError> {
        if self.checkpoint_at.is_some() || !self.chunks.is_empty() {
            return Err(ArchiveError::Invalid(
                "terrain baseline was already started",
            ));
        }
        boundary(
            &mut self.recording,
            MediaTime::ZERO,
            RecordKind::CheckpointStart,
        )?;
        self.checkpoint_at = Some(MediaTime::ZERO);
        Ok(())
    }

    /// Seals the initial scene only after every admitted baseline chunk has been written.
    pub fn end_baseline(&mut self) -> Result<(), ArchiveError> {
        let at = self
            .checkpoint_at
            .ok_or(ArchiveError::Invalid("terrain baseline is not open"))?;
        boundary(&mut self.recording, at, RecordKind::CheckpointEnd)?;
        self.checkpoint_at = None;
        Ok(())
    }

    /// Adds an indexed seek point by reusing existing content IDs, without rewriting mesh or textures.
    pub fn checkpoint(&mut self, at: MediaTime) -> Result<(), ArchiveError> {
        if self.checkpoint_at.is_some() {
            return Err(ArchiveError::Invalid("terrain baseline is still open"));
        }
        boundary(&mut self.recording, at, RecordKind::CheckpointStart)?;
        for chunk in self.chunks.values() {
            write_record(
                &mut self.recording,
                at,
                RecordKind::CheckpointPart,
                &TerrainRecord::Upsert {
                    chunk: chunk.clone(),
                },
            )?;
        }
        boundary(&mut self.recording, at, RecordKind::CheckpointEnd)
    }

    /// Drains journal publication on its worker. The capture finalizer waits for the returned task.
    pub fn finish(self, at: MediaTime) -> Result<RecordingTask, ArchiveError> {
        if self.checkpoint_at.is_some() {
            return Err(ArchiveError::Invalid(
                "cannot finish an incomplete terrain baseline",
            ));
        }
        self.recording.finish(at)
    }
}

pub struct TerrainRecording {
    reader: RecordingReader,
    loader: SceneLoader,
}

impl TerrainRecording {
    /// Transfers the validated source and object cache into an incremental worker-owned player.
    pub fn into_playback(self, maximum_bytes: usize) -> crate::playback::TerrainPlayback {
        crate::playback::TerrainPlayback::new(self.reader, self.loader, maximum_bytes)
    }

    /// Opens the native terrain schema explicitly; a raster-only recording is not a free-camera scene.
    pub fn open(parent: &Path) -> Result<Self, ArchiveError> {
        let root = parent.join(TERRAIN_DIRECTORY);
        let reader = RecordingReader::open(&root)?;
        if reader.info().scene_schema != TERRAIN_TRACK_SCHEMA {
            return Err(ArchiveError::Invalid(
                "recording has no supported native terrain track",
            ));
        }
        Ok(Self {
            reader,
            loader: SceneLoader::new(&root),
        })
    }

    /// Reconstructs a complete independent draft. Corruption, cancellation or quota failure discards it.
    /// This is a seek operation, not a replacement for incremental forward playback on every frame.
    pub fn restore(
        &mut self,
        at: MediaTime,
        maximum_bytes: usize,
        cancelled: impl Fn() -> bool,
    ) -> Result<TerrainScene, ArchiveError> {
        let mut scene = TerrainScene::new(maximum_bytes);
        let loader = &mut self.loader;
        self.reader.visit_to(at, &cancelled, |record| {
            apply_record(record, &mut scene, loader, &cancelled).map(|_| ())
        })?;
        if cancelled() {
            return Err(ArchiveError::Cancelled);
        }
        Ok(scene)
    }
}

/// A pointer hit is valid only while the exact immutable allocation is still alive.
fn cached<T: ?Sized>(cache: &VecDeque<(Weak<T>, AssetId)>, value: &Arc<T>) -> Option<AssetId> {
    cache.iter().find_map(|(weak, id)| {
        weak.upgrade()
            .filter(|old| Arc::ptr_eq(old, value))
            .map(|_| *id)
    })
}

/// Bounded weak identity history does not grow with session length or loaded chunk count.
fn remember<T: ?Sized>(cache: &mut VecDeque<(Weak<T>, AssetId)>, value: &Arc<T>, id: AssetId) {
    cache.retain(|(weak, _)| weak.strong_count() != 0);
    if cache.len() == IDENTITY_CACHE {
        cache.pop_front();
    }
    cache.push_back((Arc::downgrade(value), id));
}

/// Small content links have a shared bounded schema for journal and palette objects.
fn encode(value: &impl Serialize) -> Result<Vec<u8>, ArchiveError> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_LINK_BYTES {
        return Err(ArchiveError::Invalid(
            "native scene reference exceeds its byte limit",
        ));
    }
    Ok(bytes)
}

/// Only this worker-facing path may wait for the journal's bounded channel to drain.
fn write_record(
    recording: &mut Recording,
    at: MediaTime,
    kind: RecordKind,
    value: &impl Serialize,
) -> Result<(), ArchiveError> {
    recording.push_on_worker(RecordInput {
        at,
        kind,
        payload: encode(value)?.into(),
    })?;
    Ok(())
}

/// Multipart boundaries never carry a second ad-hoc scene format.
fn boundary(
    recording: &mut Recording,
    at: MediaTime,
    kind: RecordKind,
) -> Result<(), ArchiveError> {
    recording.push_on_worker(RecordInput {
        at,
        kind,
        payload: Arc::from([]),
    })?;
    Ok(())
}
