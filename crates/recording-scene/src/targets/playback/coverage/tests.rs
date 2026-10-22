use super::*;
use crate::targets::TargetCapture;
use recording_studio::{
    archive::{RECORDING_VERSION, RecordingInfo},
    camera::{ActorPose, CameraTargetKind},
};

/// Synthetic identities deliberately reuse one runtime ID while distinguishing spawn revisions.
fn target(revision: u64) -> CameraTarget {
    CameraTarget {
        runtime_id: 7,
        unique_id: 11,
        spawn_revision: revision,
        name: "Subject".into(),
        kind: CameraTargetKind::Player,
        entity_type: Some("minecraft:player".into()),
        pose: ActorPose {
            position: [0.0; 3],
            velocity: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            eye_offset: Some([0.0, 1.62, 0.0]),
        },
        teleported: false,
    }
}

/// The fixture uses the real indexed writer, exercising removals and lookahead across archive boundaries.
fn recording(
    samples: &[(u64, Vec<CameraTarget>)],
    duration: u64,
) -> (tempfile::TempDir, TargetRecording) {
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
    for (at, targets) in samples {
        capture.sample(MediaTime(*at), targets.clone()).unwrap();
    }
    capture.finish(MediaTime(duration)).unwrap();
    let recording = TargetRecording::open_optional(root.path())
        .unwrap()
        .unwrap();
    (root, recording)
}

/// Source intervals are half-open and are independent of preview/export output cadence.
fn request(
    start: u64,
    end: u64,
    revision: Option<u64>,
    require_eye: bool,
) -> TargetCoverageRequest {
    TargetCoverageRequest {
        directory: Default::default(),
        source_range: Span::new(MediaTime(start), MediaTime(end)).unwrap(),
        target: recording_studio::camera::CameraTargetRef {
            runtime_id: 7,
            lifetime: revision.map(|spawn_revision| recording_studio::camera::TargetLifetime {
                unique_id: 11,
                spawn_revision,
            }),
        },
        require_eye,
    }
}

#[test]
fn disappearance_between_valid_endpoints_ends_coverage_even_when_identity_is_reused() {
    let subject = target(1);
    let (_root, mut track) = recording(
        &[
            (0, vec![subject.clone()]),
            (200, vec![]),
            (300, vec![subject]),
        ],
        400,
    );
    let proof = track
        .continuous_coverage(&request(0, 400, Some(1), false), &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(proof.source_range.end, MediaTime(200));
    assert!(
        track
            .continuous_coverage(&request(250, 400, Some(1), false), &|| false)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        track
            .continuous_coverage(&request(300, 400, Some(1), false), &|| false)
            .unwrap()
            .unwrap()
            .source_range
            .end,
        MediaTime(400)
    );
}

#[test]
fn a_legacy_runtime_reference_binds_one_lifetime_and_never_tracks_its_replacement() {
    let (_root, mut track) = recording(&[(0, vec![target(1)]), (200, vec![target(2)])], 400);
    let proof = track
        .continuous_coverage(&request(0, 400, None, false), &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(proof.source_range.end, MediaTime(200));
    assert_eq!(proof.target, target(1).reference());
    assert!(
        track
            .continuous_coverage(&request(200, 400, Some(1), false), &|| false)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        track
            .continuous_coverage(&request(200, 400, Some(2), false), &|| false)
            .unwrap()
            .unwrap()
            .target,
        target(2).reference()
    );
}

#[test]
fn eye_view_admission_matches_interpolation_instead_of_only_record_boundaries() {
    let mut missing_eye = target(1);
    missing_eye.pose.eye_offset = None;
    let (_root, mut track) = recording(&[(0, vec![target(1)]), (100, vec![missing_eye])], 200);
    let proof = track
        .continuous_coverage(&request(0, 200, Some(1), true), &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(proof.source_range.end, MediaTime(1));
    assert!(
        track
            .continuous_coverage(&request(1, 200, Some(1), true), &|| false)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        track
            .continuous_coverage(&request(0, 200, Some(1), false), &|| false)
            .unwrap()
            .unwrap()
            .source_range
            .end,
        MediaTime(200)
    );
}

#[test]
fn cancelled_scan_cannot_seed_a_later_seek_or_preserve_partial_coverage() {
    let (_root, mut track) = recording(
        &[(0, vec![target(1)]), (100, vec![]), (200, vec![target(2)])],
        300,
    );
    assert!(matches!(
        track.continuous_coverage(&request(0, 300, Some(1), false), &|| true),
        Err(ArchiveError::Cancelled)
    ));
    let proof = track
        .continuous_coverage(&request(200, 500, Some(2), false), &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(
        proof.source_range,
        Span::new(MediaTime(200), MediaTime(300)).unwrap()
    );
}
