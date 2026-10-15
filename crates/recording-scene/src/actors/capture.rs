//! Actor capture admission shares immutable epochs and moves all serialization to the journal worker.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
    },
};

use recording_studio::{
    MediaTime,
    archive::{
        ArchiveError, RecordInput, RecordKind, Recording, RecordingConfig, RecordingInfo,
        RecordingState,
    },
    object_store::{AssetId, AssetWriter},
};
use render_model::{
    ActorArtworkPages, ActorRigGeometrySpan, ActorRigVertexSegments, ActorSkinResidency,
};

use super::{ACTOR_DIRECTORY, ACTOR_TRACK_SCHEMA, ActorSample, appearance, geometry, pose};

const MAX_PINNED_BYTES: usize = geometry::MAX_BYTES + appearance::MAX_BYTES;
const CHECKPOINT_MICROS: u64 = 5_000_000;

struct Charge {
    budget: Arc<AtomicUsize>,
    bytes: usize,
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.budget.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

pub struct PreparedGeometry {
    vertices: ActorRigVertexSegments,
    spans: Arc<[ActorRigGeometrySpan]>,
    _charge: Charge,
}
pub struct PreparedAppearance {
    skins: Arc<ActorSkinResidency>,
    artwork: Arc<ActorArtworkPages>,
    _charge: Charge,
}

struct Worker {
    store: AssetWriter,
    geometry: Option<(Weak<PreparedGeometry>, AssetId)>,
    appearance: Option<(Weak<PreparedAppearance>, AssetId)>,
}

pub struct ActorCapture {
    recording: Option<Recording>,
    root: PathBuf,
    worker: Arc<Mutex<Option<Worker>>>,
    pinned: Arc<AtomicUsize>,
    last_checkpoint: Option<MediaTime>,
}

impl ActorCapture {
    /// Starts the prepared actor track asynchronously, leaving gameplay independent of disk startup.
    pub fn start(parent: &Path, mut info: RecordingInfo) -> Result<Self, String> {
        info.scene_schema = ACTOR_TRACK_SCHEMA.into();
        let root = parent.join(ACTOR_DIRECTORY);
        let recording = Recording::start(
            root.clone(),
            info,
            RecordingConfig {
                disk_quota: journal_quota(),
                ..Default::default()
            },
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            recording: Some(recording),
            root,
            worker: Arc::new(Mutex::new(None)),
            pinned: Arc::default(),
            last_checkpoint: None,
        })
    }

    /// Pins one immutable geometry epoch without copying its vertex pages on the producer.
    pub fn geometry(
        &self,
        vertices: &ActorRigVertexSegments,
        spans: Arc<[ActorRigGeometrySpan]>,
    ) -> Result<Arc<PreparedGeometry>, String> {
        let mut bytes = std::mem::size_of_val(&*spans)
            + std::mem::size_of_val(&*vertices.offsets)
            + std::mem::size_of_val(&*vertices.segments);
        for page in vertices.segments.iter() {
            bytes = bytes
                .checked_add(std::mem::size_of_val(&**page))
                .ok_or("Recorded actor geometry size overflowed.")?;
        }
        if bytes > geometry::MAX_BYTES {
            return Err("Recorded actor geometry exceeded its memory budget.".into());
        }
        Ok(Arc::new(PreparedGeometry {
            vertices: vertices.clone(),
            spans,
            _charge: charge(&self.pinned, bytes)?,
        }))
    }

    /// Pins skins and artwork separately so a changing skin does not charge static geometry again.
    pub fn appearance(
        &self,
        skins: Arc<ActorSkinResidency>,
        artwork: Arc<ActorArtworkPages>,
    ) -> Result<Arc<PreparedAppearance>, String> {
        let bytes = appearance::retained_bytes(&skins, &artwork)
            .filter(|bytes| *bytes <= appearance::MAX_BYTES)
            .ok_or("Recorded actor appearance exceeded its memory budget.")?;
        Ok(Arc::new(PreparedAppearance {
            skins,
            artwork,
            _charge: charge(&self.pinned, bytes)?,
        }))
    }

    /// Queues only dynamic pose buffers; hashing, raster storage and compression run off the frame thread.
    pub fn sample(
        &mut self,
        at: MediaTime,
        sample: ActorSample,
        geometry: Arc<PreparedGeometry>,
        appearance: Arc<PreparedAppearance>,
    ) -> Result<(), String> {
        if self.last_checkpoint.is_none() && at != MediaTime::ZERO {
            return Err("The actor baseline must begin at recording time zero.".into());
        }
        if sample.rig.geometry_vertices.epoch != geometry.vertices.epoch
            || !Arc::ptr_eq(
                &sample.rig.geometry_vertices.segments,
                &geometry.vertices.segments,
            )
            || !Arc::ptr_eq(
                &sample.rig.geometry_vertices.offsets,
                &geometry.vertices.offsets,
            )
            || !Arc::ptr_eq(&sample.rig.geometry_spans, &geometry.spans)
        {
            return Err("Recorded actor geometry does not match its pinned epoch.".into());
        }
        let bytes = sample
            .retained_bytes()
            .checked_add(512)
            .ok_or("Recorded actor sample size overflowed.")?;
        let checkpoint = self
            .last_checkpoint
            .is_none_or(|old| at.0.saturating_sub(old.0) >= CHECKPOINT_MICROS);
        let recording = self
            .recording
            .as_mut()
            .ok_or("Actor recording is finishing.")?;
        if checkpoint {
            boundary(recording, at, RecordKind::CheckpointStart)?;
        }
        let root = self.root.clone();
        let worker = Arc::clone(&self.worker);
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
                    // This lock is acquired only by the existing journal worker, never by the producer.
                    let mut context = worker
                        .lock()
                        .map_err(|_| ArchiveError::Invalid("recorded actor worker was poisoned"))?;
                    if context.is_none() {
                        *context = Some(Worker {
                            store: AssetWriter::open(
                                &root,
                                RecordingConfig::default().disk_quota - journal_quota(),
                            )?,
                            geometry: None,
                            appearance: None,
                        });
                    }
                    let context = context.as_mut().expect("prepared actor storage");
                    let geometry_id = match cached(&context.geometry, &geometry) {
                        Some(id) => id,
                        None => {
                            let id = geometry::store(
                                &mut context.store,
                                &geometry.vertices,
                                &geometry.spans,
                            )?;
                            context.geometry = Some((Arc::downgrade(&geometry), id));
                            id
                        }
                    };
                    let appearance_id = match cached(&context.appearance, &appearance) {
                        Some(id) => id,
                        None => {
                            let id = appearance::store(
                                &mut context.store,
                                &appearance.skins,
                                &appearance.artwork,
                            )?;
                            context.appearance = Some((Arc::downgrade(&appearance), id));
                            id
                        }
                    };
                    pose::write(out, geometry_id, appearance_id, &sample)
                },
            )
            .map_err(|error| error.to_string())?;
        if checkpoint {
            boundary(recording, at, RecordKind::CheckpointEnd)?;
            self.last_checkpoint = Some(at);
        }
        Ok(())
    }

    /// Reports asynchronous failures even when no actor pose changes or gameplay is paused.
    pub fn error(&self) -> Option<String> {
        self.recording.as_ref()?.task().progress()?.error
    }

    /// Drains all accepted samples only from the capture finalization worker.
    pub fn finish(mut self, at: MediaTime) -> Result<(), String> {
        let progress = self
            .recording
            .take()
            .ok_or("Actor recording was already closed.")?
            .finish(at)
            .map_err(|error| error.to_string())?
            .wait();
        if progress.state != RecordingState::Complete {
            return Err(progress
                .error
                .unwrap_or_else(|| "Actor storage did not finish.".into()));
        }
        Ok(())
    }
}

/// Reserves only retained immutable allocations; queue bytes are charged separately by the archive.
fn charge(pinned: &Arc<AtomicUsize>, bytes: usize) -> Result<Charge, String> {
    pinned
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
            old.checked_add(bytes)
                .filter(|total| *total <= MAX_PINNED_BYTES)
        })
        .map_err(|_| "Recorded actor assets exceeded their pinned memory budget.".to_owned())?;
    Ok(Charge {
        budget: Arc::clone(pinned),
        bytes,
    })
}

/// A weak identity cache never extends the lifetime of obsolete skin or geometry epochs.
fn cached<T>(entry: &Option<(Weak<T>, AssetId)>, current: &Arc<T>) -> Option<AssetId> {
    entry.as_ref().and_then(|(old, id)| {
        old.upgrade()
            .filter(|old| Arc::ptr_eq(old, current))
            .map(|_| *id)
    })
}

/// Journal and immutable-object budgets share one track's declared disk allowance.
fn journal_quota() -> u64 {
    RecordingConfig::default().disk_quota / 2
}

/// All parts of a checkpoint retain one media timestamp and the archive's existing validation.
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
