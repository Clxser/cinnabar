use super::*;
use recording_studio::{
    archive::{RECORDING_VERSION, RecordingInfo},
    camera::CameraPose,
    capture::{CaptureSession, CaptureSettings},
};

#[test]
fn every_native_track_stops_replay_only_and_allows_pov_only_when_a_video_was_requested() {
    for &track in NativeTrack::ALL {
        for mode in [CaptureMode::ReplayOnly, CaptureMode::ReplayAndVideo] {
            let mut failures = NativeFailures::default();
            assert_eq!(failures.admission(mode, false), Admission::Continue);
            failures.record(
                mode,
                track,
                MediaTime(18_000),
                "Synthetic native failure".into(),
            );
            assert_eq!(
                failures.admission(mode, false),
                if mode.records_video() {
                    Admission::Continue
                } else {
                    Admission::StopAt(MediaTime(18_000))
                }
            );
            assert_eq!(
                failures
                    .message()
                    .unwrap()
                    .contains("Video recording continues"),
                mode.records_video()
            );
            let metadata = track.metadata(failures.tracks.get(&track));
            assert!(!metadata.complete);
            assert!(metadata.error.as_deref().unwrap().contains("0.018s"));
            metadata.validate().unwrap();
        }
    }
}

#[test]
fn a_primary_or_video_failure_still_closes_admission_after_an_optional_native_failure() {
    for mode in [CaptureMode::ReplayOnly, CaptureMode::ReplayAndVideo] {
        let mut failures = NativeFailures::default();
        assert_eq!(failures.admission(mode, true), Admission::Stop);
        failures.record(
            mode,
            NativeTrack::Terrain,
            MediaTime(18_000),
            "Native queue is full".into(),
        );
        assert_eq!(
            failures.admission(mode, true),
            if mode.records_video() {
                Admission::Stop
            } else {
                Admission::StopAt(MediaTime(18_000))
            }
        );
    }
}

#[test]
fn later_failures_never_extend_native_coverage_or_replace_the_original_cause() {
    let mut failures = NativeFailures::default();
    failures.record(
        CaptureMode::ReplayAndVideo,
        NativeTrack::Terrain,
        MediaTime(18_000),
        "First cause".into(),
    );
    failures.record(
        CaptureMode::ReplayAndVideo,
        NativeTrack::Terrain,
        MediaTime(90_000),
        "Later cause".into(),
    );
    failures.record(
        CaptureMode::ReplayAndVideo,
        NativeTrack::Actors,
        MediaTime(30_000),
        "Independent actors failure".into(),
    );
    let original = failures.tracks.get(&NativeTrack::Terrain).unwrap();
    assert_eq!(original.at, MediaTime(18_000));
    assert_eq!(original.error, "First cause");
    assert_eq!(
        failures.admission(CaptureMode::ReplayOnly, false),
        Admission::StopAt(MediaTime(18_000))
    );
    assert!(
        !NativeTrack::Actors
            .metadata(failures.tracks.get(&NativeTrack::Actors))
            .complete
    );
    assert!(NativeTrack::Environment.metadata(None).complete);
}

#[test]
fn native_track_registry_has_distinct_owner_schemas_and_directories() {
    let mut schemas = std::collections::BTreeSet::new();
    let mut directories = std::collections::BTreeSet::new();
    for &track in NativeTrack::ALL {
        let metadata = track.metadata(None);
        metadata.validate().unwrap();
        assert!(schemas.insert(metadata.schema));
        assert!(directories.insert(metadata.directory));
    }
}

/// A real native-only primary journal makes admission/stop state testable without carriers or FFmpeg.
fn active() -> (tempfile::TempDir, Active) {
    let root = tempfile::tempdir().unwrap();
    let info = RecordingInfo {
        version: RECORDING_VERSION,
        name: "Synthetic capture".into(),
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
        .camera(MediaTime::ZERO, CameraPose::default())
        .unwrap();
    let active = Active {
        session,
        audio: None,
        terrain: None,
        native_failures: Default::default(),
        environment: None,
        actors: None,
        targets: None,
        visuals: None,
        started: 10.0,
        identity: (1, 0, 7),
        stopping: None,
        video_readbacks: Default::default(),
        captured: Default::default(),
        last_frame: Some(0),
        last_camera_frame: Some(0),
        last_attack: 0,
        last_combat: 0,
        failure: None,
    };
    (root, active)
}

#[test]
fn native_producer_failure_closes_active_admission_at_its_exclusive_sample_boundary() {
    let (_root, mut active) = active();
    interrupt(
        &mut active,
        NativeTrack::Terrain,
        "Synthetic terrain admission error".into(),
        10.018,
    );
    assert!(active.failure.as_ref().unwrap().contains("terrain"));
    assert_eq!(active.stopping.unwrap().1, MediaTime(18_000));
    assert!(active.native_failures.contains(NativeTrack::Terrain));
    fatal(&mut active, "A later independent failure".into(), 10.050);
    assert_eq!(active.stopping.unwrap().1, MediaTime(18_000));
    assert!(
        active
            .session
            .finish(MediaTime(18_000), None, 0, 0)
            .is_err()
    );
}

#[test]
fn independent_primary_failure_stops_even_when_no_native_track_has_failed() {
    let (_root, mut active) = active();
    fatal(
        &mut active,
        "Synthetic primary or video failure".into(),
        10.018,
    );
    assert_eq!(
        active.failure.as_deref(),
        Some("Synthetic primary or video failure")
    );
    assert_eq!(active.stopping.unwrap().1, MediaTime(33_333));
    assert!(!active.native_failures.contains(NativeTrack::Terrain));
    assert!(
        active
            .session
            .finish(MediaTime(33_333), None, 0, 0)
            .is_err()
    );
}
