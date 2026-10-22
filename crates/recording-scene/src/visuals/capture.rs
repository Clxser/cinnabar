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

use super::{FrozenVisuals, VISUAL_DIRECTORY, VISUAL_TRACK_SCHEMA, retention, store_frame};

pub struct VisualCapture {
    disk: recording_studio::disk::DiskBudget,
    recording: Option<Recording>,
    root: PathBuf,
    store: Arc<Mutex<Option<Worker>>>,
    retained: retention::Budget,
    last_checkpoint: Option<MediaTime>,
}

struct QueuedVisuals {
    _lease: retention::Lease,
    frame: FrozenVisuals,
}

impl QueuedVisuals {
    /// Borrowing the complete owner keeps its retained-byte lease live throughout serialization.
    fn write(&self, out: &mut dyn std::io::Write, worker: &mut Worker) -> Result<(), ArchiveError> {
        store_frame(
            out,
            &mut worker.store,
            &mut worker.hand,
            &mut worker.atmosphere,
            &mut worker.items,
            &mut worker.nametags,
            &self.frame,
        )
    }
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
    pub fn start(parent: &Path, info: RecordingInfo) -> Result<Self, String> {
        Self::start_with_disk(
            parent,
            info,
            recording_studio::disk::DiskBudget::new(RecordingConfig::default().disk_quota),
        )
    }

    /// Native visual journals and object staging share every other stream's take-wide allowance.
    pub fn start_with_disk(
        parent: &Path,
        mut info: RecordingInfo,
        disk: recording_studio::disk::DiskBudget,
    ) -> Result<Self, String> {
        info.scene_schema = VISUAL_TRACK_SCHEMA.into();
        let root = parent.join(VISUAL_DIRECTORY);
        let recording = Recording::start_with_disk(
            root.clone(),
            info,
            RecordingConfig {
                disk_quota: disk.maximum(),
                queued_bytes: retention::MAX_QUEUED_BYTES,
                compression_level: 0,
                ..Default::default()
            },
            disk.clone(),
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            disk,
            recording: Some(recording),
            root,
            store: Arc::new(Mutex::new(None)),
            retained: retention::Budget::default(),
            last_checkpoint: None,
        })
    }

    /// Bounds every retained allocation before moving immutable draws into the nonblocking queue.
    pub fn sample(&mut self, at: MediaTime, frame: FrozenVisuals) -> Result<(), String> {
        if self.last_checkpoint.is_none() && at != MediaTime::ZERO {
            return Err("The prepared visual baseline must begin at time zero.".into());
        }
        let (lease, bytes) = self.retained.retain(&frame)?;
        let queued = QueuedVisuals {
            _lease: lease,
            frame,
        };
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
        let disk = self.disk.clone();
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
                            store: AssetWriter::open_with_disk(
                                &root,
                                disk.maximum(),
                                disk.clone(),
                            )?,
                            hand: super::hand::GeometryCache::default(),
                            atmosphere: super::atmosphere::AssetCache::default(),
                            items: super::items::Cache::default(),
                            nametags: super::nametags::Cache::default(),
                        });
                    }
                    let worker = store.as_mut().expect("prepared visual storage");
                    queued.write(out, worker)
                },
            )
            .map_err(|error| format!("Prepared visual recording: {error}"))?;
        if checkpoint {
            boundary(recording, at, RecordKind::CheckpointEnd)?;
            self.last_checkpoint = Some(at);
        }
        Ok(())
    }

    /// Storage errors remain visible even if gameplay stops publishing changes.
    pub fn error(&self) -> Option<String> {
        self.recording
            .as_ref()?
            .task()
            .progress()?
            .error
            .map(|error| format!("Prepared visual recording: {error}"))
    }

    /// Only the capture finalizer drains accepted draws and waits for storage completion.
    pub fn finish(mut self, at: MediaTime) -> Result<(), String> {
        let recording = self
            .recording
            .take()
            .ok_or("Prepared visual recording is already closed.")?;
        let task = recording.task();
        let progress = match recording.finish(at) {
            Ok(task) => task.wait(),
            Err(error) => return Err(task.wait().error.unwrap_or_else(|| error.to_string())),
        };
        if progress.state != RecordingState::Complete {
            return Err(progress
                .error
                .unwrap_or_else(|| "Prepared visual storage did not finish.".into()));
        }
        Ok(())
    }
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
        .map_err(|error| format!("Prepared visual recording: {error}"))
}
