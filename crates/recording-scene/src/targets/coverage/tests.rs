use super::*;
use crate::targets::{TARGET_DIRECTORY, TARGET_TRACK_SCHEMA, TargetCapture};
use recording_studio::{
    MediaTime, Span,
    archive::{RECORDING_VERSION, RecordingInfo},
    camera::{ActorPose, CameraTarget, CameraTargetKind},
    capture::{CaptureSession, CaptureSettings, CaptureTrack},
};

/// Both source owners write real journals; no renderer, codec executable or game carrier is required.
fn take(mismatch: bool) -> (tempfile::TempDir, TargetCoverageRequest) {
    let root = tempfile::tempdir().unwrap();
    let info = RecordingInfo {
        version: RECORDING_VERSION,
        name: "Synthetic take".into(),
        game_version: "fixture".into(),
        protocol_version: 0,
        scene_schema: "fixture".into(),
        created_unix_ms: 0,
        viewport: [16, 16],
        local_actor: 7,
    };
    let mut session =
        CaptureSession::create(root.path(), info, CaptureSettings::default()).unwrap();
    session
        .camera(
            MediaTime::ZERO,
            recording_studio::camera::CameraPose::default(),
        )
        .unwrap();
    let mut target_info = session.info().clone();
    if mismatch {
        target_info.created_unix_ms += 1;
    }
    let mut targets = TargetCapture::start(session.directory(), target_info).unwrap();
    let subject = CameraTarget {
        runtime_id: 7,
        unique_id: 11,
        spawn_revision: 1,
        name: "Subject".into(),
        kind: CameraTargetKind::Player,
        entity_type: Some("minecraft:player".into()),
        teleported: false,
        pose: ActorPose {
            position: [0.0; 3],
            velocity: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            eye_offset: Some([0.0, 1.62, 0.0]),
        },
    };
    targets
        .sample(MediaTime::ZERO, vec![subject.clone()])
        .unwrap();
    targets.finish(MediaTime(2_000_000)).unwrap();
    session
        .scene_track(CaptureTrack {
            schema: TARGET_TRACK_SCHEMA.into(),
            directory: TARGET_DIRECTORY.into(),
            complete: true,
            error: None,
        })
        .unwrap();
    let summary = session.finish(MediaTime(1_000_000), None, 0, 0).unwrap();
    let request = TargetCoverageRequest {
        directory: summary.directory,
        source_range: Span::new(MediaTime::ZERO, MediaTime(2_000_000)).unwrap(),
        target: subject.reference(),
        require_eye: false,
    };
    (root, request)
}

#[test]
fn source_admission_clamps_target_coverage_to_the_parent_recorded_camera_duration() {
    let (_root, request) = take(false);
    let coverage = verify_coverage(&request, &Cancellation::default())
        .unwrap()
        .unwrap();
    assert_eq!(coverage.source_range.end, MediaTime(1_000_000));
    assert_eq!(coverage.target, request.target);
}

#[test]
fn a_present_subject_track_from_another_take_is_an_error_not_empty_coverage() {
    let (_root, request) = take(true);
    assert!(
        verify_coverage(&request, &Cancellation::default())
            .unwrap_err()
            .contains("different recorded take")
    );
}

#[test]
fn unavailable_subject_tracks_and_cancelled_requests_remain_explicit() {
    let (_root, request) = take(false);
    std::fs::rename(
        request.directory.join(TARGET_DIRECTORY),
        request.directory.join("unavailable-target-fixture"),
    )
    .unwrap();
    assert!(
        verify_coverage(&request, &Cancellation::default())
            .unwrap_err()
            .contains("no recorded subject track")
    );
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(
        verify_coverage(&request, &cancellation)
            .unwrap_err()
            .contains("cancelled")
    );
}
