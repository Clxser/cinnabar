//! Actor capture admission shares immutable epochs and moves all serialization to the journal worker.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};

use recording_studio::{
    MediaTime,
    archive::{
        ArchiveError, RecordInput, RecordKind, Recording, RecordingConfig, RecordingInfo,
        RecordingState, SCENE_CHECKPOINT_INTERVAL,
    },
    object_store::{AssetId, AssetWriter},
};
use render_model::{
    ActorArtworkPages, ActorRigGeometrySpan, ActorRigVertexSegments, ActorSkinResidency,
};

use super::{ACTOR_DIRECTORY, ACTOR_TRACK_SCHEMA, ActorSample, appearance, geometry, pose};

mod budget;

pub struct PreparedGeometry {
    // Release charges before their retained allocations can be freed and their addresses reused.
    _charge: budget::Lease,
    vertices: ActorRigVertexSegments,
    spans: Arc<[ActorRigGeometrySpan]>,
}
pub struct PreparedAppearance {
    _charge: budget::Lease,
    skins: Arc<ActorSkinResidency>,
    artwork: Arc<ActorArtworkPages>,
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
    pinned: budget::Budget,
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
            pinned: budget::Budget::default(),
            last_checkpoint: None,
        })
    }

    /// Pins one immutable geometry epoch without copying its vertex pages on the producer.
    pub fn geometry(
        &mut self,
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
            _charge: self.pinned.geometry(vertices, &spans)?,
            vertices: vertices.clone(),
            spans,
        }))
    }

    /// Pins skins and artwork separately so a changing skin does not charge static geometry again.
    pub fn appearance(
        &mut self,
        skins: Arc<ActorSkinResidency>,
        artwork: Arc<ActorArtworkPages>,
    ) -> Result<Arc<PreparedAppearance>, String> {
        appearance::retained_bytes(&skins, &artwork)
            .filter(|bytes| *bytes <= appearance::MAX_BYTES)
            .ok_or("Recorded actor appearance exceeded its memory budget.")?;
        Ok(Arc::new(PreparedAppearance {
            _charge: self.pinned.appearance(&skins, &artwork)?,
            skins,
            artwork,
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
            .is_none_or(|old| at.0.saturating_sub(old.0) >= SCENE_CHECKPOINT_INTERVAL.0);
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
