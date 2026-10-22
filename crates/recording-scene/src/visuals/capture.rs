//! Prepared visual samples use a bounded archive worker and immutable pixel references.

use recording_studio::{
    MediaTime,
    archive::{
        ArchiveError, RecordInput, RecordKind, Recording, RecordingConfig, RecordingInfo,
        RecordingState, SCENE_CHECKPOINT_INTERVAL,
    },
    object_store::AssetWriter,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use super::{FrozenVisuals, VISUAL_DIRECTORY, VISUAL_TRACK_SCHEMA, store_frame};

pub struct VisualCapture {
    recording: Option<Recording>,
    root: PathBuf,
    store: Arc<Mutex<Option<Worker>>>,
    last_checkpoint: Option<MediaTime>,
}

struct Worker {
    store: AssetWriter,
    hand: super::hand::GeometryCache,
    atmosphere: super::atmosphere::AssetCache,
    items: super::items::Cache,
    nametags: super::nametags::Cache,
}

impl VisualCapture {
    /// Startup and all raster hashing happen on archive workers; capture admission never waits on disk.
    pub fn start(parent: &Path, mut info: RecordingInfo) -> Result<Self, String> {
        info.scene_schema = VISUAL_TRACK_SCHEMA.into();
        let root = parent.join(VISUAL_DIRECTORY);
        let recording = Recording::start(
            root.clone(),
            info,
            RecordingConfig {
                disk_quota: journal_quota(),
                queued_bytes: 256 * 1024 * 1024,
                compression_level: 0,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            recording: Some(recording),
            root,
            store: Arc::new(Mutex::new(None)),
            last_checkpoint: None,
        })
    }

    /// Bounds every retained allocation before moving immutable draws into the nonblocking queue.
    pub fn sample(&mut self, at: MediaTime, frame: FrozenVisuals) -> Result<(), String> {
        if self.last_checkpoint.is_none() && at != MediaTime::ZERO {
            return Err("The prepared visual baseline must begin at time zero.".into());
        }
        let bytes = frame
            .retained_bytes()
            .checked_add(512)
            .filter(|&bytes| bytes <= 192 * 1024 * 1024)
            .ok_or("The prepared visual frame exceeds its memory budget.")?;
        let checkpoint = self
            .last_checkpoint
            .is_none_or(|old| at.0.saturating_sub(old.0) >= SCENE_CHECKPOINT_INTERVAL.0);
        let recording = self
            .recording
            .as_mut()
            .ok_or("Prepared visual recording is finishing.")?;
        if checkpoint {
            boundary(recording, at, RecordKind::CheckpointStart)?;
        }
        let root = self.root.clone();
        let store = Arc::clone(&self.store);
        recording
            .push_prepared(
                at,
                if checkpoint {
                    RecordKind::CheckpointPart
                } else {
                    RecordKind::Scene
                },
                bytes,
                move |out| {
                    let mut store = store.lock().map_err(|_| {
                        ArchiveError::Invalid("prepared visual worker was poisoned")
                    })?;
                    if store.is_none() {
                        *store = Some(Worker {
                            store: AssetWriter::open(
                                &root,
                                RecordingConfig::default().disk_quota - journal_quota(),
                            )?,
                            hand: super::hand::GeometryCache::default(),
                            atmosphere: super::atmosphere::AssetCache::default(),
                            items: super::items::Cache::default(),
                            nametags: super::nametags::Cache::default(),
                        });
                    }
                    let worker = store.as_mut().expect("prepared visual storage");
                    store_frame(
                        out,
                        &mut worker.store,
                        &mut worker.hand,
                        &mut worker.atmosphere,
                        &mut worker.items,
                        &mut worker.nametags,
                        &frame,
                    )
                },
            )
            .map_err(|e| e.to_string())?;
        if checkpoint {
            boundary(recording, at, RecordKind::CheckpointEnd)?;
            self.last_checkpoint = Some(at);
        }
        Ok(())
    }

    /// Storage errors remain visible even if gameplay stops publishing changes.
    pub fn error(&self) -> Option<String> {
        self.recording.as_ref()?.task().progress()?.error
    }

    /// Only the capture finalizer drains accepted draws and waits for storage completion.
    pub fn finish(mut self, at: MediaTime) -> Result<(), String> {
        let progress = self
            .recording
            .take()
            .ok_or("Prepared visual recording is already closed.")?
            .finish(at)
            .map_err(|e| e.to_string())?
            .wait();
        if progress.state != RecordingState::Complete {
            return Err(progress
                .error
                .unwrap_or_else(|| "Prepared visual storage did not finish.".into()));
        }
        Ok(())
    }
}

/// Pixel objects and journal segments share the track's declared disk allowance.
fn journal_quota() -> u64 {
    RecordingConfig::default().disk_quota / 2
}

/// Checkpoint framing shares the same timestamp and archive admission as its visual sample.
fn boundary(recording: &mut Recording, at: MediaTime, kind: RecordKind) -> Result<(), String> {
    recording
        .push(RecordInput {
            at,
            kind,
            payload: Arc::from([]),
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}
