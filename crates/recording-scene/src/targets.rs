//! Small received-actor camera targets, independent of the original view's prepared draw coverage.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use recording_studio::{
    MediaTime,
    archive::{
        ArchiveError, RecordInput, RecordKind, Recording, RecordingConfig, RecordingCursor,
        RecordingInfo, RecordingReader, RecordingState, SCENE_CHECKPOINT_INTERVAL,
    },
    camera::{CameraTarget, MAX_CAMERA_TARGETS},
};

mod codec;

pub const TARGET_TRACK_SCHEMA: &str = "cinnabar.camera-targets/1";
pub const TARGET_DIRECTORY: &str = "camera-targets.cinreplay";

#[derive(Clone, Debug, PartialEq)]
pub struct TargetFrame {
    pub at: MediaTime,
    pub targets: Arc<[CameraTarget]>,
}

impl TargetFrame {
    /// Sorted identities keep target lookups independent of the number of timeline camera shots.
    pub fn target(&self, runtime_id: u64) -> Option<&CameraTarget> {
        self.targets
            .binary_search_by_key(&runtime_id, |target| target.runtime_id)
            .ok()
            .map(|index| &self.targets[index])
    }
}

struct Delta {
    upserts: Vec<CameraTarget>,
    removed: Vec<u64>,
}

pub struct TargetCapture {
    recording: Option<Recording>,
    previous: Vec<CameraTarget>,
    previous_at: Option<MediaTime>,
    checkpoint: Option<MediaTime>,
}

impl TargetCapture {
    /// Starts its archive worker without touching the filesystem on the capture producer.
    pub fn start(parent: &Path, mut info: RecordingInfo) -> Result<Self, String> {
        info.scene_schema = TARGET_TRACK_SCHEMA.into();
        Ok(Self {
            recording: Some(
                Recording::start(
                    parent.join(TARGET_DIRECTORY),
                    info,
                    RecordingConfig::default(),
                )
                .map_err(|error| error.to_string())?,
            ),
            previous: Vec::new(),
            previous_at: None,
            checkpoint: None,
        })
    }

    /// Records changed targets/removals and returns the old sample vector for producer scratch reuse.
    pub fn sample(
        &mut self,
        at: MediaTime,
        mut targets: Vec<CameraTarget>,
    ) -> Result<Vec<CameraTarget>, String> {
        if targets.len() > MAX_CAMERA_TARGETS
            || self.previous_at.is_some_and(|old| at < old)
            || self.previous_at.is_none() && at != MediaTime::ZERO
        {
            return Err("Camera target capture exceeds its count or timestamp bounds.".into());
        }
        targets.sort_unstable_by_key(|target| target.runtime_id);
        if targets
            .windows(2)
            .any(|pair| pair[0].runtime_id == pair[1].runtime_id)
        {
            return Err("Duplicate recorded camera target identity.".into());
        }
        let mut old_index = 0;
        let checkpoint = self
            .checkpoint
            .is_none_or(|old| at.0.saturating_sub(old.0) >= SCENE_CHECKPOINT_INTERVAL.0);
        let mut delta = Delta {
            upserts: Vec::new(),
            removed: Vec::new(),
        };
        for target in &mut targets {
            while old_index < self.previous.len()
                && self.previous[old_index].runtime_id < target.runtime_id
            {
                if !checkpoint {
                    delta.removed.push(self.previous[old_index].runtime_id);
                }
                old_index += 1;
            }
            let old = self
                .previous
                .get(old_index)
                .filter(|old| old.runtime_id == target.runtime_id);
            target.pose.velocity = [0.0; 3];
            if let Some(old) = old.filter(|old| old.same_lifetime(target) && !target.teleported)
                && let Some(previous) = self.previous_at.filter(|previous| *previous < at)
            {
                let seconds = at.saturating_sub(previous).seconds();
                target.pose.velocity = std::array::from_fn(|axis| {
                    (target.pose.position[axis] - old.pose.position[axis]) / seconds
                });
            }
            target.validate().map_err(|error| error.to_string())?;
            if checkpoint || old.is_none_or(|old| old != &*target) {
                delta.upserts.push(target.clone());
            }
            if old.is_some() {
                old_index += 1;
            }
        }
        if !checkpoint {
            delta.removed.extend(
                self.previous[old_index..]
                    .iter()
                    .map(|target| target.runtime_id),
            );
        }
        if checkpoint || !delta.upserts.is_empty() || !delta.removed.is_empty() {
            let recording = self
                .recording
                .as_mut()
                .ok_or("Camera target recording is finishing.")?;
            if checkpoint {
                boundary(recording, at, RecordKind::CheckpointStart)?;
            }
            let retained = delta.upserts.capacity() * std::mem::size_of::<CameraTarget>()
                + delta.removed.capacity() * std::mem::size_of::<u64>()
                + delta
                    .upserts
                    .iter()
                    .map(|target| target.name.len())
                    .sum::<usize>();
            recording
                .push_prepared(
                    at,
                    if checkpoint {
                        RecordKind::CheckpointPart
                    } else {
                        RecordKind::Scene
                    },
                    retained,
                    move |output| codec::write(output, &delta),
                )
                .map_err(|error| error.to_string())?;
            if checkpoint {
                boundary(recording, at, RecordKind::CheckpointEnd)?;
                self.checkpoint = Some(at);
            }
        }
        let recycled = std::mem::replace(&mut self.previous, targets);
        self.previous_at = Some(at);
        Ok(recycled)
    }

    /// Reports asynchronous storage errors even while all recorded actors stand still.
    pub fn error(&self) -> Option<String> {
        self.recording.as_ref()?.task().progress()?.error
    }

    /// Drains accepted target changes only on the recording's existing finalization worker.
    pub fn finish(mut self, at: MediaTime) -> Result<(), String> {
        let progress = self
            .recording
            .take()
            .ok_or("Camera target recording is already closed.")?
            .finish(at)
            .map_err(|error| error.to_string())?
            .wait();
        if progress.state == RecordingState::Complete {
            Ok(())
        } else {
            Err(progress
                .error
                .unwrap_or_else(|| "Camera target recording did not finish.".into()))
        }
    }
}

pub struct TargetRecording {
    cursor: RecordingCursor,
    state: BTreeMap<u64, CameraTarget>,
    current: Option<TargetFrame>,
}

impl TargetRecording {
    /// Older takes can lack targets. A present but invalid target directory is an error, not a fallback.
    pub fn open_optional(parent: &Path) -> Result<Option<Self>, ArchiveError> {
        let root = parent.join(TARGET_DIRECTORY);
        match std::fs::symlink_metadata(&root) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(ArchiveError::Invalid(
                    "camera target track is not a directory",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(ArchiveError::Io { path: root, source }),
        }
        let reader = RecordingReader::open(&root)?;
        if reader.info().scene_schema != TARGET_TRACK_SCHEMA {
            return Err(ArchiveError::Invalid(
                "unsupported camera target track schema",
            ));
        }
        Ok(Some(Self {
            cursor: reader.into_cursor(),
            state: BTreeMap::new(),
            current: None,
        }))
    }

    pub fn info(&self) -> &RecordingInfo {
        self.cursor.reader().info()
    }
    pub fn duration(&self) -> MediaTime {
        self.cursor.reader().duration()
    }

    /// Checkpoint seeks reconstruct a private target set before publishing its sorted immutable array.
    pub fn seek(
        &mut self,
        at: MediaTime,
        cancelled: &impl Fn() -> bool,
    ) -> Result<TargetFrame, ArchiveError> {
        self.update(at, true, cancelled)
    }

    /// Forward playback consumes only crossed deltas and reuses unchanged published target arrays.
    pub fn advance_to(
        &mut self,
        at: MediaTime,
        cancelled: &impl Fn() -> bool,
    ) -> Result<TargetFrame, ArchiveError> {
        self.update(at, false, cancelled)
    }

    /// A failed sibling source invalidates this draft until a complete scene seek succeeds.
    pub fn invalidate(&mut self) {
        self.cursor.invalidate();
        self.state.clear();
        self.current = None;
    }

    /// Invalid/cancelled drafts cannot seed a later frame after the renderer retained an older publication.
    fn update(
        &mut self,
        at: MediaTime,
        seek: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<TargetFrame, ArchiveError> {
        if seek {
            self.state.clear();
        }
        let state = &mut self.state;
        let mut changed = seek;
        let mut apply = |record: recording_studio::archive::ReplayRecord<'_>| {
            match record.kind {
                RecordKind::CheckpointStart => {
                    state.clear();
                    changed = true;
                }
                RecordKind::CheckpointEnd => {}
                RecordKind::CheckpointPart | RecordKind::Scene => {
                    let delta = codec::read(&mut &record.payload[..])?;
                    if record.kind == RecordKind::CheckpointPart && !delta.removed.is_empty() {
                        return Err(ArchiveError::Invalid("target checkpoint contains removals"));
                    }
                    for id in delta.removed {
                        state.remove(&id);
                    }
                    for target in delta.upserts {
                        if state.len() == MAX_CAMERA_TARGETS
                            && !state.contains_key(&target.runtime_id)
                        {
                            return Err(ArchiveError::Quota);
                        }
                        state.insert(target.runtime_id, target);
                    }
                    changed = true;
                }
                _ => return Err(ArchiveError::Invalid("unexpected camera target record")),
            }
            Ok(())
        };
        let result = if seek {
            self.cursor.seek(at, cancelled, &mut apply)
        } else {
            self.cursor.advance_to(at, cancelled, &mut apply)
        };
        if let Err(error) = result {
            self.invalidate();
            return Err(error);
        }
        if cancelled() {
            self.invalidate();
            return Err(ArchiveError::Cancelled);
        }
        if changed {
            self.current = Some(TargetFrame {
                at,
                targets: self.state.values().cloned().collect::<Vec<_>>().into(),
            });
        }
        self.current.clone().ok_or(ArchiveError::Invalid(
            "camera targets need a successful baseline",
        ))
    }
}

/// Target snapshots use the same indexed multipart checkpoint framing as the other scene tracks.
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
