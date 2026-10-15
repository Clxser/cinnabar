//! Recorded original-view camera playback, independent of the live world's current camera.

use std::path::Path;

use serde::Deserialize;

use super::{CaptureSettings, POV_CHECKPOINT_VERSION, POV_SCENE_SCHEMA, PovCheckpoint};
use crate::{
    MediaTime,
    archive::{
        ArchiveError, RecordKind, RecordingCursor, RecordingInfo, RecordingReader, ReplayRecord,
    },
    camera::CameraPose,
};

const MAX_CAMERA_RECORD_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug)]
pub struct RecordedView {
    pub at: MediaTime,
    pub pose: CameraPose,
}

pub struct CameraRecording {
    cursor: RecordingCursor,
    current: Option<RecordedView>,
}

impl CameraRecording {
    /// Opens only POV capture metadata; an arbitrary journal cannot masquerade as a recorded view.
    pub fn open(parent: &Path) -> Result<Self, ArchiveError> {
        let reader = RecordingReader::open(parent)?;
        if reader.info().scene_schema != POV_SCENE_SCHEMA {
            return Err(ArchiveError::Invalid(
                "recording has no supported original-view camera track",
            ));
        }
        Ok(Self {
            cursor: reader.into_cursor(),
            current: None,
        })
    }

    /// Scene owners compare take metadata before combining separately recorded tracks.
    pub fn info(&self) -> &RecordingInfo {
        self.cursor.reader().info()
    }

    /// Includes the encoder's finalized frame boundary when the parent capture completed.
    pub fn duration(&self) -> MediaTime {
        self.cursor.reader().duration()
    }

    /// Uses the nearest camera checkpoint, including legacy initial settings-only checkpoints.
    pub fn restore(
        &mut self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
    ) -> Result<RecordedView, ArchiveError> {
        let mut current = None;
        let result = self
            .cursor
            .seek(at, &cancelled, |record| observe(record, &mut current))
            .and_then(|()| {
                current.ok_or(ArchiveError::Invalid(
                    "recording has no camera sample at this time",
                ))
            });
        match result {
            Ok(view) => {
                self.current = Some(view);
                Ok(view)
            }
            Err(error) => {
                self.invalidate();
                Err(error)
            }
        }
    }

    /// Advances only newly crossed samples, preserving the original camera between recorded frames.
    pub fn advance_to(
        &mut self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
    ) -> Result<RecordedView, ArchiveError> {
        let mut current = self.current;
        let result = self
            .cursor
            .advance_to(at, &cancelled, |record| observe(record, &mut current))
            .and_then(|()| {
                current.ok_or(ArchiveError::Invalid(
                    "camera playback needs a successful seek",
                ))
            });
        match result {
            Ok(view) => {
                self.current = Some(view);
                Ok(view)
            }
            Err(error) => {
                self.invalidate();
                Err(error)
            }
        }
    }

    /// A failed sibling scene track cannot leave an independently advancing camera cursor behind.
    pub fn invalidate(&mut self) {
        self.cursor.invalidate();
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Checkpoint {
    Current(PovCheckpoint),
    Initial(CaptureSettings),
}

/// Only camera payloads affect view state; committed combat evidence remains owned by edit analysis.
fn observe(
    record: ReplayRecord<'_>,
    current: &mut Option<RecordedView>,
) -> Result<(), ArchiveError> {
    match record.kind {
        RecordKind::CheckpointStart => *current = None,
        RecordKind::CheckpointEnd | RecordKind::Gameplay => {}
        RecordKind::CheckpointPart => {
            bounded(record.payload)?;
            match serde_json::from_slice::<Checkpoint>(record.payload)? {
                Checkpoint::Current(checkpoint) => {
                    if checkpoint.version != POV_CHECKPOINT_VERSION {
                        return Err(ArchiveError::Invalid(
                            "unsupported POV camera checkpoint version",
                        ));
                    }
                    checkpoint
                        .settings
                        .validate()
                        .map_err(|_| ArchiveError::Invalid("invalid captured camera settings"))?;
                    if let Some(pose) = checkpoint.camera {
                        admit_pose(pose, record.at, current)?;
                    }
                }
                Checkpoint::Initial(settings) => settings
                    .validate()
                    .map_err(|_| ArchiveError::Invalid("invalid legacy camera checkpoint"))?,
            }
        }
        RecordKind::Camera => {
            bounded(record.payload)?;
            admit_pose(serde_json::from_slice(record.payload)?, record.at, current)?;
        }
        _ => {
            return Err(ArchiveError::Invalid(
                "unexpected record in original-view camera track",
            ));
        }
    }
    Ok(())
}

/// Fixed-schema camera data cannot consume the generic journal's much larger record allowance.
fn bounded(payload: &[u8]) -> Result<(), ArchiveError> {
    if payload.len() > MAX_CAMERA_RECORD_BYTES {
        return Err(ArchiveError::Invalid(
            "recorded camera metadata exceeds its bounds",
        ));
    }
    Ok(())
}

/// Original-view replay never substitutes a default camera for a missing or malformed sample.
fn admit_pose(
    pose: CameraPose,
    at: MediaTime,
    current: &mut Option<RecordedView>,
) -> Result<(), ArchiveError> {
    pose.validate()
        .map_err(|_| ArchiveError::Invalid("invalid recorded camera pose"))?;
    *current = Some(RecordedView { at, pose });
    Ok(())
}
