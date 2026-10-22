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

/// Blocks the first checkpoint's serialization while its end and a later scene are already admitted.
fn admitted_checkpoint_survives_rejection(
    reject: impl FnOnce(&mut Recording) -> Result<u64, ArchiveError>,
) {
    let root = tempfile::tempdir().unwrap();
    let tail = [
        RecordInput {
            at: MediaTime::ZERO,
            kind: RecordKind::CheckpointEnd,
            payload: Arc::from([]),
        },
        RecordInput {
            at: MediaTime(100_000),
            kind: RecordKind::Scene,
            payload: Arc::from(&b"accepted scene"[..]),
        },
    ];
    let mut recording = Recording::start_ready(
        root.path().join("take"),
        info(),
        RecordingConfig {
            queued_records: tail.len(),
            ..Default::default()
        },
    )
    .unwrap();
    recording
        .push(RecordInput {
            at: MediaTime::ZERO,
            kind: RecordKind::CheckpointStart,
            payload: Arc::from([]),
        })
        .unwrap();
    let (entered, worker_entered) = mpsc::sync_channel(0);
    let (release, worker_release) = mpsc::sync_channel(0);
    recording
        .push_prepared(
            MediaTime::ZERO,
            RecordKind::CheckpointPart,
            1,
            move |output| {
                entered.send(()).unwrap();
                worker_release.recv().unwrap();
                output
                    .write_all(b"accepted baseline")
                    .map_err(|_| ArchiveError::Invalid("fixture encode failed"))
            },
        )
        .unwrap();
    worker_entered.recv().unwrap();
    for record in tail {
        recording.push(record).unwrap();
    }
    let rejected = reject(&mut recording).unwrap_err().to_string();
    let task = recording.task();
    assert_eq!(task.progress().unwrap().state, RecordingState::Failed);
    assert!(!task.is_finished());
    assert!(recording.finish(MediaTime(200_000)).is_err());
    release.send(()).unwrap();
    let progress = task.wait();
    assert_eq!(progress.state, RecordingState::Failed);
    assert_eq!(progress.error.as_deref(), Some(rejected.as_str()));
    assert_eq!(progress.processed_records, 4);
    assert_eq!(progress.committed_records, 4);
    assert_eq!(task.progress().unwrap().queued_bytes, 0);
    let reader = RecordingReader::open(task.path()).unwrap();
    assert!(reader.recovery().interrupted);
    assert_eq!(reader.duration(), MediaTime(100_000));
    let mut records = Vec::new();
    reader
        .visit_to(
            reader.duration(),
            || false,
            |record| {
                records.push((record.sequence, record.kind, record.payload.to_vec()));
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(
        records,
        vec![
            (1, RecordKind::CheckpointStart, vec![]),
            (2, RecordKind::CheckpointPart, b"accepted baseline".to_vec()),
            (3, RecordKind::CheckpointEnd, vec![]),
            (4, RecordKind::Scene, b"accepted scene".to_vec()),
        ]
    );
}

#[test]
fn invalid_next_admission_preserves_the_accepted_checkpoint_end_and_scene_tail() {
    admitted_checkpoint_survives_rejection(|recording| {
        let result = recording.push(RecordInput {
            at: MediaTime(MAX_MEDIA_MICROS + 1),
            kind: RecordKind::Scene,
            payload: Arc::from([]),
        });
        assert!(matches!(&result, Err(ArchiveError::Invalid(_))));
        result
    });
}

#[test]
fn full_next_admission_preserves_the_accepted_checkpoint_end_and_scene_tail() {
    admitted_checkpoint_survives_rejection(|recording| {
        let result = recording.push(RecordInput {
            at: MediaTime(200_000),
            kind: RecordKind::Scene,
            payload: Arc::from(&b"rejected scene"[..]),
        });
        assert!(matches!(&result, Err(ArchiveError::Backpressure)));
        result
    });
}

#[test]
fn fatal_storage_encoding_error_stops_before_the_dependent_accepted_tail() {
    let root = tempfile::tempdir().unwrap();
    let mut recording =
        Recording::start_ready(root.path().join("take"), info(), RecordingConfig::default())
            .unwrap();
    baseline(&mut recording);
    let (entered, worker_entered) = mpsc::sync_channel(0);
    let (release, worker_release) = mpsc::sync_channel(0);
    recording
        .push_prepared(MediaTime(100_000), RecordKind::Scene, 1, move |_| {
            entered.send(()).unwrap();
            worker_release.recv().unwrap();
            Err(ArchiveError::Invalid("fixture storage encoding failed"))
        })
        .unwrap();
    worker_entered.recv().unwrap();
    let tail_encoded = Arc::new(AtomicBool::new(false));
    let worker_tail = Arc::clone(&tail_encoded);
    recording
        .push_prepared(MediaTime(200_000), RecordKind::Scene, 1, move |_| {
            worker_tail.store(true, Ordering::Release);
            Ok(())
        })
        .unwrap();
    let task = recording.finish(MediaTime(300_000)).unwrap();
    release.send(()).unwrap();
    let progress = task.wait();
    assert_eq!(progress.state, RecordingState::Failed);
    assert_eq!(
        progress.error.as_deref(),
        Some("fixture storage encoding failed")
    );
    assert!(!tail_encoded.load(Ordering::Acquire));
    assert_eq!(task.progress().unwrap().queued_bytes, 0);
    let reader = RecordingReader::open(task.path()).unwrap();
    assert_eq!(reader.duration(), MediaTime::ZERO);
    assert!(reader.recovery().interrupted);
}
