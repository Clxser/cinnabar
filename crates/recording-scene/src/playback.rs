//! Worker-owned native scene playback. The renderer consumes admitted patches, never a live authority.

use std::collections::BTreeSet;

use recording_studio::{
    MediaTime,
    archive::{ArchiveError, RecordKind, RecordingCursor, RecordingReader},
};

use crate::{
    journal::apply_record,
    scene::{ChunkId, SceneChunk, SceneLoader, TerrainScene},
};

mod scene;
pub use scene::{SceneFrame, ScenePlayback};

/// A reset replaces the viewport's terrain; ordinary updates touch only changed chunk bindings.
pub struct TerrainPatch {
    pub reset: bool,
    pub upserts: Vec<SceneChunk>,
    pub removed: Vec<ChunkId>,
}

pub struct TerrainPlayback {
    cursor: RecordingCursor,
    loader: SceneLoader,
    scene: TerrainScene,
    changed: BTreeSet<ChunkId>,
}

impl TerrainPlayback {
    /// Reuses the source's validated index and object cache without reopening or cloning the world.
    pub(crate) fn new(reader: RecordingReader, loader: SceneLoader, maximum_bytes: usize) -> Self {
        Self {
            cursor: reader.into_cursor(),
            loader,
            scene: TerrainScene::new(maximum_bytes),
            changed: BTreeSet::new(),
        }
    }

    /// Last successfully admitted state; an invalidated draft is never exposed to a viewport.
    pub fn scene(&self) -> Option<&TerrainScene> {
        self.cursor.position().map(|_| &self.scene)
    }

    /// Frozen completion bounds also cover recoverable, interrupted takes.
    pub fn duration(&self) -> MediaTime {
        self.cursor.reader().duration()
    }

    /// Native tracks share one take identity even though each owns a different scene schema.
    pub fn info(&self) -> &recording_studio::archive::RecordingInfo {
        self.cursor.reader().info()
    }

    /// Seeks from a complete baseline and returns a replacement patch only after all object admission succeeds.
    pub fn seek(
        &mut self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
    ) -> Result<TerrainPatch, ArchiveError> {
        self.scene.clear();
        self.changed.clear();
        let scene = &mut self.scene;
        let loader = &mut self.loader;
        let result = self.cursor.seek(at, &cancelled, |record| {
            apply_record(record, scene, loader, &cancelled).map(|_| ())
        });
        self.finish(result, &cancelled)?;
        Ok(TerrainPatch {
            reset: true,
            upserts: self.scene.chunks().cloned().collect(),
            removed: Vec::new(),
        })
    }

    /// Forward play applies deltas once. Redundant later terrain checkpoints never clear or rebuild the world.
    pub fn advance_to(
        &mut self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
    ) -> Result<TerrainPatch, ArchiveError> {
        self.changed.clear();
        let scene = &mut self.scene;
        let loader = &mut self.loader;
        let changed = &mut self.changed;
        let result = self.cursor.advance_to(at, &cancelled, |record| {
            if matches!(
                record.kind,
                RecordKind::CheckpointStart
                    | RecordKind::CheckpointPart
                    | RecordKind::CheckpointEnd
            ) {
                return Ok(());
            }
            if let Some(key) = apply_record(record, scene, loader, &cancelled)? {
                changed.insert(key);
            }
            Ok(())
        });
        self.finish(result, &cancelled)?;
        let mut patch = TerrainPatch {
            reset: false,
            upserts: Vec::new(),
            removed: Vec::new(),
        };
        for &key in &self.changed {
            match self.scene.chunk(key) {
                Some(chunk) => patch.upserts.push(chunk.clone()),
                None => patch.removed.push(key),
            }
        }
        Ok(patch)
    }

    /// A sibling track's failure prevents a half-updated native scene from becoming a later frame.
    pub fn invalidate(&mut self) {
        self.cursor.invalidate();
        self.scene.clear();
        self.changed.clear();
    }

    /// Error and cancellation discard private staging state while the renderer retains its last good patch.
    fn finish(
        &mut self,
        result: Result<(), ArchiveError>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), ArchiveError> {
        let result = if cancelled() {
            Err(ArchiveError::Cancelled)
        } else {
            result
        };
        if result.is_err() {
            self.invalidate();
        }
        result
    }
}
