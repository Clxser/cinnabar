use std::{path::Path, sync::Arc};

use crate::{
    Id, MediaTime, PlaybackRate, Project, Span,
    archive::{RECORDING_VERSION, RecordInput, RecordKind, Recording, RecordingInfo},
    automation::{GameplayEvent, GameplayEventKind},
    capture::{CaptureSettings, POV_CHECKPOINT_VERSION, POV_SCENE_SCHEMA, PovCheckpoint},
    model::{Clip, Framing, MediaKind, MediaSource},
};

/// Synthetic primary journals use the production writer and checkpoint encoding without game carriers.
pub(super) fn recording(root: &Path) -> Recording {
    let mut recording = Recording::start_ready(
        root.join("take"),
        RecordingInfo {
            version: RECORDING_VERSION,
            name: "Companion recovery fixture".into(),
            game_version: "fixture".into(),
            protocol_version: 0,
            scene_schema: POV_SCENE_SCHEMA.into(),
            created_unix_ms: 0,
            viewport: [16, 16],
            local_actor: 7,
        },
        Default::default(),
    )
    .unwrap();
    checkpoint(&mut recording, MediaTime::ZERO);
    recording
}

/// The second complete checkpoint exposes accidental latest-checkpoint-only event recovery.
pub(super) fn checkpoint(recording: &mut Recording, at: MediaTime) {
    for (kind, payload) in [
        (RecordKind::CheckpointStart, Vec::new()),
        (
            RecordKind::CheckpointPart,
            serde_json::to_vec(&PovCheckpoint {
                version: POV_CHECKPOINT_VERSION,
                settings: CaptureSettings::default(),
                camera: Some(Default::default()),
            })
            .unwrap(),
        ),
        (RecordKind::CheckpointEnd, Vec::new()),
    ] {
        recording
            .push(RecordInput {
                at,
                kind,
                payload: Arc::from(payload),
            })
            .unwrap();
    }
}

/// Payload and container timestamps can differ deliberately when exercising malformed journals.
pub(super) fn event(
    recording: &mut Recording,
    container_at: MediaTime,
    sequence: u64,
    at: MediaTime,
    kind: GameplayEventKind,
) {
    recording
        .push(RecordInput {
            at: container_at,
            kind: RecordKind::Gameplay,
            payload: Arc::from(serde_json::to_vec(&GameplayEvent { sequence, at, kind }).unwrap()),
        })
        .unwrap();
}

/// This initial project models the output of native scene verification, independently of companions.
pub(super) fn project(parent: &Path, prefix: MediaTime) -> Project {
    let mut project = Project::new("Recovered native fixture");
    project.export.size = [16, 16];
    project.export.include_audio = false;
    project.sources.push(MediaSource {
        id: Id(1),
        name: "Verified native source".into(),
        path: parent.to_path_buf(),
        kind: MediaKind::Replay,
        duration: prefix,
        size: [16, 16],
        frame_rate: Some(Default::default()),
        sha256: None,
    });
    project.clips.push(Clip {
        id: Id(2),
        source: Id(1),
        source_range: Span::new(MediaTime::ZERO, prefix).unwrap(),
        at: MediaTime::ZERO,
        rate: PlaybackRate::default(),
        speed_ramp: None,
        track: 0,
        framing: Framing::default(),
        enabled: true,
    });
    project
}

/// A manual bookmark is independently meaningful even when capture-time combat gaps are unavailable.
pub(super) fn bookmark(label: &str) -> GameplayEventKind {
    GameplayEventKind::Bookmark {
        label: label.into(),
    }
}
