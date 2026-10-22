//! Bounded prepared-environment admission, sharing the journal's existing IO/compression worker.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
    },
};

use assets::RuntimeAtmosphereAssets;
use recording_studio::{
    MediaTime,
    archive::{
        ArchiveError, RecordInput, RecordKind, Recording, RecordingConfig, RecordingInfo,
        RecordingState, SCENE_CHECKPOINT_INTERVAL,
    },
    object_store::{AssetId, AssetWriter},
};

use super::{
    ENVIRONMENT_DIRECTORY, ENVIRONMENT_TRACK_SCHEMA, EnvironmentState, Skyboxes, sky_assets,
    write_frame,
};

const MAX_PINNED_BYTES: usize = 256 * 1024 * 1024;
const JOURNAL_BYTES: u64 = 32 * 1024 * 1024;

struct Charge {
    budget: Arc<AtomicUsize>,
    bytes: usize,
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.budget.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

pub struct PreparedEnvironmentAssets {
    atmosphere: Arc<RuntimeAtmosphereAssets>,
    skyboxes: Option<Arc<Skyboxes>>,
    _charge: Charge,
}

struct WorkerAssets {
    store: AssetWriter,
    previous: Option<(Weak<PreparedEnvironmentAssets>, AssetId)>,
}

pub struct EnvironmentCapture {
    recording: Option<Recording>,
    root: PathBuf,
    worker: Arc<Mutex<Option<WorkerAssets>>>,
    pinned: Arc<AtomicUsize>,
    last_checkpoint: Option<MediaTime>,
}

impl EnvironmentCapture {
    /// Starts storage asynchronously; callers supply prepared values and their existing recording clock.
    pub fn start(parent: &Path, mut info: RecordingInfo) -> Result<Self, String> {
        info.scene_schema = ENVIRONMENT_TRACK_SCHEMA.into();
        let root = parent.join(ENVIRONMENT_DIRECTORY);
        let recording = Recording::start(
            root.clone(),
            info,
            RecordingConfig {
                disk_quota: JOURNAL_BYTES,
                ..Default::default()
            },
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            recording: Some(recording),
            root,
            worker: Arc::new(Mutex::new(None)),
            pinned: Arc::new(AtomicUsize::new(0)),
            last_checkpoint: None,
        })
    }

    /// Pins one immutable material epoch once, independently of how many state samples reference it.
    pub fn assets(
        &self,
        atmosphere: Arc<RuntimeAtmosphereAssets>,
        skyboxes: Option<Arc<Skyboxes>>,
    ) -> Result<Arc<PreparedEnvironmentAssets>, String> {
        let mut bytes = atmosphere.recording_retained_bytes();
        if let Some(skyboxes) = &skyboxes {
            bytes = bytes
                .checked_add(std::mem::size_of_val(&**skyboxes))
                .ok_or("Environment asset size overflowed.")?;
            for skybox in skyboxes.iter() {
                for face in skybox.faces.iter().flatten() {
                    bytes = bytes
                        .checked_add(face.rgba8.len())
                        .ok_or("Skybox asset size overflowed.")?;
                }
            }
        }
        self.pinned
            .try_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                old.checked_add(bytes)
                    .filter(|&total| total <= MAX_PINNED_BYTES)
            })
            .map_err(|_| "Recorded sky assets exceeded their memory budget.".to_owned())?;
        Ok(Arc::new(PreparedEnvironmentAssets {
            atmosphere,
            skyboxes,
            _charge: Charge {
                budget: Arc::clone(&self.pinned),
                bytes,
            },
        }))
    }

    /// A small immutable state sample queues no pixel copies, hashing, serialization or disk work here.
    pub fn sample(
        &mut self,
        at: MediaTime,
        state: EnvironmentState,
        assets: Arc<PreparedEnvironmentAssets>,
    ) -> Result<(), String> {
        if self.last_checkpoint.is_none() && at != MediaTime::ZERO {
            return Err("The environment baseline must begin at recording time zero.".into());
        }
        let checkpoint = self
            .last_checkpoint
            .is_none_or(|old| at.0.saturating_sub(old.0) >= SCENE_CHECKPOINT_INTERVAL.0);
        let recording = self
            .recording
            .as_mut()
            .ok_or("Environment recording is finishing.")?;
        if checkpoint {
            boundary(recording, at, RecordKind::CheckpointStart)?;
        }
        let root = self.root.clone();
        let worker = Arc::clone(&self.worker);
        let kind = if checkpoint {
            RecordKind::CheckpointPart
        } else {
            RecordKind::Scene
        };
        recording
            .push_prepared(
                at,
                kind,
                std::mem::size_of::<EnvironmentState>() + 512,
                move |out| {
                    // Only this journal worker locks the context; no producer or render path acquires it.
                    let mut context = worker.lock().map_err(|_| {
                        ArchiveError::Invalid("recorded environment worker state was poisoned")
                    })?;
                    if context.is_none() {
                        let quota = RecordingConfig::default()
                            .disk_quota
                            .saturating_sub(JOURNAL_BYTES);
                        *context = Some(WorkerAssets {
                            store: AssetWriter::open(&root, quota)?,
                            previous: None,
                        });
                    }
                    let context = context.as_mut().expect("prepared environment storage");
                    let cached = context.previous.as_ref().and_then(|(old, id)| {
                        old.upgrade()
                            .filter(|old| Arc::ptr_eq(old, &assets))
                            .map(|_| *id)
                    });
                    let id = match cached {
                        Some(id) => id,
                        None => {
                            let id = sky_assets::store(
                                &mut context.store,
                                &assets.atmosphere,
                                assets.skyboxes.as_deref(),
                            )?;
                            context.previous = Some((Arc::downgrade(&assets), id));
                            id
                        }
                    };
                    write_frame(out, id, state)
                },
            )
            .map_err(|error| error.to_string())?;
        if checkpoint {
            boundary(recording, at, RecordKind::CheckpointEnd)?;
            self.last_checkpoint = Some(at);
        }
        Ok(())
    }

    /// Surfaces asynchronous storage failures even while the camera and environment remain unchanged.
    pub fn error(&self) -> Option<String> {
        self.recording.as_ref()?.task().progress()?.error
    }

    /// Closes admission, drains accepted samples and waits only on the capture finalization worker.
    pub fn finish(mut self, at: MediaTime) -> Result<(), String> {
        let progress = self
            .recording
            .take()
            .ok_or("Environment recording was already closed.")?
            .finish(at)
            .map_err(|error| error.to_string())?
            .wait();
        if progress.state != RecordingState::Complete {
            return Err(progress
                .error
                .unwrap_or_else(|| "Environment storage did not finish.".into()));
        }
        Ok(())
    }
}

/// Multipart state checkpoints use the same validated framing as larger terrain snapshots.
fn boundary(recording: &mut Recording, at: MediaTime, kind: RecordKind) -> Result<(), String> {
    recording
        .push(RecordInput {
            at,
            kind,
            payload: Arc::from([]),
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
}
