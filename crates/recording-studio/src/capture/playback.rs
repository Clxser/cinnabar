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
    next: Option<RecordedView>,
    position: Option<MediaTime>,
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
            next: None,
            position: None,
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
        self.current = None;
        self.next = None;
        let result = (|| {
            let mut baseline = at;
            loop {
                self.cursor.seek(baseline, &cancelled, |record| {
                    observe(record, &mut self.current)
                })?;
                if self.current.is_some() {
                    break;
                }
                let checkpoint = self.cursor.reader().checkpoint_at(baseline)?;
                if checkpoint == MediaTime::ZERO {
                    break;
                }
                baseline = MediaTime(checkpoint.0 - 1);
            }
            if baseline < at {
                self.cursor
                    .advance_to(at, &cancelled, |record| observe(record, &mut self.current))?;
            }
            self.read_next(&cancelled)?;
            self.sample(at)
        })();
        self.finish(at, result)
    }

    /// Promotes crossed samples once and evaluates two admitted poses without accumulating motion.
    pub fn advance_to(
        &mut self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
    ) -> Result<RecordedView, ArchiveError> {
        let result = (|| {
            if cancelled() {
                return Err(ArchiveError::Cancelled);
            }
            if self.position.is_none_or(|previous| at < previous) || at > self.duration() {
                return Err(ArchiveError::Invalid(
                    "camera playback needs a successful seek",
                ));
            }
            if self.next.is_some_and(|next| next.at <= at) {
                self.current = self.next.take();
                self.cursor
                    .advance_to(at, &cancelled, |record| observe(record, &mut self.current))?;
                self.read_next(&cancelled)?;
            }
            self.sample(at)
        })();
        self.finish(at, result)
    }

    /// A failed sibling scene track cannot leave an independently advancing camera cursor behind.
    pub fn invalidate(&mut self) {
        self.cursor.invalidate();
        self.current = None;
        self.next = None;
        self.position = None;
    }

    /// Gameplay records and legacy settings-only checkpoints never manufacture a new camera sample.
    fn read_next(&mut self, cancelled: &impl Fn() -> bool) -> Result<(), ArchiveError> {
        self.next = None;
        while let Some(at) = self.cursor.next_time(cancelled)? {
            self.cursor
                .advance_to(at, cancelled, |record| observe(record, &mut self.next))?;
            if self.next.is_some() {
                break;
            }
        }
        Ok(())
    }

    /// Source camera rotation follows shortest arcs; large unmarked jumps remain discrete cuts.
    fn sample(&self, at: MediaTime) -> Result<RecordedView, ArchiveError> {
        let current = self.current.ok_or(ArchiveError::Invalid(
            "recording has no camera sample at this time",
        ))?;
        let pose = self
            .next
            .filter(|next| next.at > current.at)
            .map_or(current.pose, |next| {
                let distance = current
                    .pose
                    .position
                    .into_iter()
                    .zip(next.pose.position)
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f64>();
                if distance > 64.0_f64.powi(2) || next.at.0 - current.at.0 > 1_000_000 {
                    return current.pose;
                }
                let amount = at.saturating_sub(current.at).0 as f64
                    / next.at.saturating_sub(current.at).0 as f64;
                current.pose.interpolated(next.pose, amount)
            });
        Ok(RecordedView { at, pose })
    }

    /// Failed lookahead cannot leave a future cursor paired with an old published source pose.
    fn finish(
        &mut self,
        at: MediaTime,
        result: Result<RecordedView, ArchiveError>,
    ) -> Result<RecordedView, ArchiveError> {
        if result.is_ok() {
            self.position = Some(at);
        } else {
            self.invalidate();
        }
        result
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
        RecordKind::CheckpointStart => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_only_checkpoints_preserve_the_previous_camera_pose() {
        let pose = CameraPose {
            yaw: 137.0,
            ..CameraPose::default()
        };
        let mut current = Some(RecordedView {
            at: MediaTime(10),
            pose,
        });
        observe(
            ReplayRecord {
                sequence: 1,
                at: MediaTime(20),
                kind: RecordKind::CheckpointStart,
                payload: &[],
            },
            &mut current,
        )
        .unwrap();
        let legacy = serde_json::to_vec(&CaptureSettings::default()).unwrap();
        observe(
            ReplayRecord {
                sequence: 2,
                at: MediaTime(20),
                kind: RecordKind::CheckpointPart,
                payload: &legacy,
            },
            &mut current,
        )
        .unwrap();
        assert_eq!(current.unwrap().pose, pose);
        assert_eq!(current.unwrap().at, MediaTime(10));
        let no_pose = serde_json::to_vec(&PovCheckpoint {
            version: POV_CHECKPOINT_VERSION,
            settings: CaptureSettings::default(),
            camera: None,
        })
        .unwrap();
        observe(
            ReplayRecord {
                sequence: 3,
                at: MediaTime(30),
                kind: RecordKind::CheckpointPart,
                payload: &no_pose,
            },
            &mut current,
        )
        .unwrap();
        assert_eq!(current.unwrap().pose, pose);
        assert_eq!(current.unwrap().at, MediaTime(10));
    }
}
