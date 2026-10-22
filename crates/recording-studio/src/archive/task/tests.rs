use super::*;
use crate::{
    archive::{RECORDING_VERSION, RecordKind, RecordingReader},
    time::MAX_MEDIA_MICROS,
};

/// Metadata fixtures need no game carrier; the channel barrier controls physical worker lifetime.
fn info() -> RecordingInfo {
    RecordingInfo {
        version: RECORDING_VERSION,
        name: "Worker completion fixture".into(),
        game_version: "fixture".into(),
        protocol_version: 0,
        scene_schema: "fixture/1".into(),
        created_unix_ms: 0,
        viewport: [16, 16],
        local_actor: 1,
    }
}

/// A committed baseline makes a later interrupted scene record independently recoverable.
fn baseline(recording: &mut Recording) {
    for kind in [
        RecordKind::CheckpointStart,
        RecordKind::CheckpointPart,
        RecordKind::CheckpointEnd,
    ] {
        recording
            .push(RecordInput {
                at: MediaTime::ZERO,
                kind,
                payload: Arc::from([]),
            })
            .unwrap();
    }
}

#[test]
fn producer_failure_does_not_report_physical_completion_while_storage_is_barrier_blocked() {
    let root = tempfile::tempdir().unwrap();
    let mut recording =
        Recording::start_ready(root.path().join("take"), info(), RecordingConfig::default())
            .unwrap();
    baseline(&mut recording);
    let (entered, worker_entered) = mpsc::sync_channel(0);
    let (release, worker_release) = mpsc::sync_channel(0);
    recording
        .push_prepared(MediaTime(100_000), RecordKind::Scene, 1, move |output| {
            entered.send(()).unwrap();
            worker_release.recv().unwrap();
            output
                .write_all(b"scene")
                .map_err(|_| ArchiveError::Invalid("fixture encode failed"))
        })
        .unwrap();
    worker_entered.recv().unwrap();
    let task = recording.task();
    assert!(
        recording
            .push(RecordInput {
                at: MediaTime(MAX_MEDIA_MICROS + 1),
                kind: RecordKind::Scene,
                payload: Arc::from([])
            })
            .is_err()
    );
    assert_eq!(task.progress().unwrap().state, RecordingState::Failed);
    assert!(!task.is_finished());
    assert!(recording.finish(MediaTime(200_000)).is_err());
    release.send(()).unwrap();
    let progress = task.wait();
    assert!(task.is_finished());
    assert_eq!(progress.state, RecordingState::Failed);
    assert_eq!(
        RecordingReader::open(task.path()).unwrap().duration(),
        MediaTime(100_000)
    );
}

#[test]
fn encoder_panic_still_signals_physical_completion_and_preserves_the_failure() {
    let root = tempfile::tempdir().unwrap();
    let mut recording =
        Recording::start_ready(root.path().join("take"), info(), RecordingConfig::default())
            .unwrap();
    baseline(&mut recording);
    recording
        .push_prepared(MediaTime(100_000), RecordKind::Scene, 1, |_| {
            panic!("synthetic encoder panic")
        })
        .unwrap();
    let task = recording.task();
    let _ = recording.finish(MediaTime(200_000));
    let progress = task.wait();
    assert!(task.is_finished());
    assert_eq!(progress.state, RecordingState::Failed);
    assert!(progress.error.unwrap().contains("unexpectedly"));
}
