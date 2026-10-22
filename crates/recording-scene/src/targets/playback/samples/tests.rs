use super::*;
use crate::targets::TargetCapture;
use recording_studio::{
    archive::{RECORDING_VERSION, RecordingInfo},
    camera::CameraTargetKind,
};

/// Spawn revisions deliberately share one runtime and unique ID to exercise lifetime binding.
fn subject(revision: u64) -> CameraTarget {
    CameraTarget {
        runtime_id: 7,
        unique_id: 11,
        spawn_revision: revision,
        name: "Subject".into(),
        kind: CameraTargetKind::Player,
        entity_type: Some("minecraft:player".into()),
        view: Default::default(),
        teleported: false,
        pose: ActorPose {
            position: [0.0; 3],
            velocity: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            eye_offset: Some([0.0, 1.62, 0.0]),
        },
    }
}

/// Small real journals reproduce boundary traversal without a renderer or runtime recording fixtures.
fn track(changes: &[(u64, Vec<CameraTarget>)]) -> (tempfile::TempDir, TargetRecording) {
    let root = tempfile::tempdir().unwrap();
    let info = RecordingInfo {
        version: RECORDING_VERSION,
        name: "Synthetic targets".into(),
        game_version: "fixture".into(),
        protocol_version: 0,
        scene_schema: "fixture".into(),
        created_unix_ms: 0,
        viewport: [16, 16],
        local_actor: 7,
    };
    let mut capture = TargetCapture::start(root.path(), info).unwrap();
    for (at, subjects) in changes {
        capture.sample(MediaTime(*at), subjects.clone()).unwrap();
    }
    capture.finish(MediaTime(1_000_000)).unwrap();
    let recording = TargetRecording::open_optional(root.path())
        .unwrap()
        .unwrap();
    (root, recording)
}

#[test]
fn sample_admission_ignores_unseen_gaps_but_requires_every_requested_target() {
    let (root, recording) = track(&[
        (0, vec![subject(1)]),
        (25_000, vec![]),
        (75_000, vec![subject(1)]),
    ]);
    let samples = [
        Ok(MediaTime(0)),
        Ok(MediaTime(100_000)),
        Ok(MediaTime(200_000)),
    ];
    assert_eq!(
        recording
            .verify_samples(
                samples,
                subject(1).reference(),
                false,
                MediaTime(1_000_000),
                &Cancellation::default()
            )
            .unwrap(),
        Some(subject(1).reference())
    );
    let recording = TargetRecording::open_optional(root.path())
        .unwrap()
        .unwrap();
    let samples = [
        Ok(MediaTime(0)),
        Ok(MediaTime(50_000)),
        Ok(MediaTime(100_000)),
    ];
    assert_eq!(
        recording
            .verify_samples(
                samples,
                subject(1).reference(),
                false,
                MediaTime(1_000_000),
                &Cancellation::default()
            )
            .unwrap(),
        None
    );
}

#[test]
fn one_subject_eye_admission_matches_full_preview_pose_interpolation() {
    let mut missing_eye = subject(1);
    missing_eye.pose.eye_offset = None;
    let (root, recording) = track(&[
        (0, vec![subject(1)]),
        (110_000, vec![missing_eye]),
        (200_000, vec![subject(1)]),
    ]);
    let mut preview = TargetRecording::open_optional(root.path())
        .unwrap()
        .unwrap();
    let times = [MediaTime(0), MediaTime(100_000), MediaTime(200_000)];
    let eyes: Vec<_> = times
        .iter()
        .map(|at| {
            preview
                .seek(*at, &|| false)
                .unwrap()
                .target(7)
                .unwrap()
                .pose
                .eye_offset
                .is_some()
        })
        .collect();
    assert_eq!(eyes, [true, false, true]);
    assert_eq!(
        recording
            .verify_samples(
                times.into_iter().map(Ok),
                subject(1).reference(),
                true,
                MediaTime(1_000_000),
                &Cancellation::default()
            )
            .unwrap(),
        None
    );
}

#[test]
fn sampled_runtime_reuse_is_rejected_after_binding_the_first_legacy_reference() {
    let (_root, recording) = track(&[(0, vec![subject(1)]), (75_000, vec![subject(2)])]);
    let reference = CameraTargetRef {
        lifetime: None,
        ..subject(1).reference()
    };
    assert_eq!(
        recording
            .verify_samples(
                [Ok(MediaTime(0)), Ok(MediaTime(100_000))],
                reference,
                false,
                MediaTime(1_000_000),
                &Cancellation::default()
            )
            .unwrap(),
        None
    );
}
