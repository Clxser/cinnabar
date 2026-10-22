use super::*;
use recording_scene::targets::{TARGET_DIRECTORY, TARGET_TRACK_SCHEMA, TargetCapture};
use recording_studio::{
    Id, PlaybackRate, Project,
    archive::{RECORDING_VERSION, RecordingInfo},
    camera::{ActorPose, CameraPose, CameraRigKind, CameraShot, CameraTarget, CameraTargetKind},
    capture::{CaptureSession, CaptureSettings, CaptureTrack},
    effect::Easing,
    export::Destination,
    model::{Clip, Framing, MediaSource, Transition, TransitionKind},
    retime::{SpeedKey, SpeedRamp},
};
use std::sync::Arc;

/// Every sample retains protocol identity independently of display name or runtime-ID reuse.
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
            position: [0.0, 64.0, 4.0],
            velocity: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            eye_offset: Some([0.0, 1.62, 0.0]),
        },
    }
}

/// Real metadata and target journals exercise admission without a graphics device or video encoder.
fn take(changes: &[(u64, Option<u64>)]) -> (tempfile::TempDir, Project) {
    take_subjects(
        changes
            .iter()
            .map(|&(at, revision)| (at, revision.map(subject).into_iter().collect())),
    )
}

/// Target pose fixtures also exercise native eye interpolation independently of lifetime changes.
fn take_subjects(
    changes: impl IntoIterator<Item = (u64, Vec<CameraTarget>)>,
) -> (tempfile::TempDir, Project) {
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
    let mut capture =
        CaptureSession::create(root.path(), info, CaptureSettings::default()).unwrap();
    capture
        .camera(MediaTime::ZERO, CameraPose::default())
        .unwrap();
    let mut targets = TargetCapture::start(capture.directory(), capture.info().clone()).unwrap();
    for (at, subjects) in changes {
        targets.sample(MediaTime(at), subjects).unwrap();
    }
    let duration = MediaTime(1_000_000);
    targets.finish(duration).unwrap();
    capture
        .scene_track(CaptureTrack {
            schema: TARGET_TRACK_SCHEMA.into(),
            directory: TARGET_DIRECTORY.into(),
            complete: true,
            error: None,
        })
        .unwrap();
    let summary = capture.finish(duration, None, 0, 0).unwrap();
    let mut project = Project::new("Sampled subject export");
    project.export.fps = FrameRate {
        numerator: 10,
        denominator: 1,
    };
    project.sources.push(MediaSource {
        id: Id(1),
        name: "Native take".into(),
        path: summary.directory,
        kind: MediaKind::Replay,
        duration,
        size: [16, 16],
        frame_rate: None,
        sha256: None,
    });
    project.clips.push(Clip {
        id: Id(2),
        source: Id(1),
        source_range: interval(0, 300_000),
        at: MediaTime::ZERO,
        rate: PlaybackRate::default(),
        speed_ramp: None,
        track: 0,
        framing: Framing::default(),
        enabled: true,
    });
    project.shots.push(
        CameraShot::from_preview(
            Id(3),
            interval(0, 300_000),
            CameraRigKind::Track,
            CameraPose::default(),
            Some(&subject(1)),
        )
        .unwrap(),
    );
    (root, project)
}

/// Only camera-subject admission runs; terrain/device/encoder admission has separate owners.
fn admit(project: Project) -> Result<(), String> {
    prepare(
        &ExportRequest {
            project: Arc::new(project),
            source_base: Default::default(),
            destination: Destination::Directory(Default::default()),
        },
        &Cancellation::default(),
    )
}

#[test]
fn gaps_entirely_between_output_frames_are_accepted_without_weakening_suggestion_proofs() {
    let (_root, project) = take(&[(0, Some(1)), (25_000, None), (75_000, Some(1))]);
    let query = recording_studio::camera::coverage::TargetCoverageRequest {
        directory: project.sources[0].path.clone(),
        source_range: interval(0, 200_001),
        target: subject(1).reference(),
        require_eye: false,
    };
    let continuous = recording_scene::targets::verify_coverage(&query, &Cancellation::default())
        .unwrap()
        .unwrap();
    assert_eq!(continuous.source_range.end, MediaTime(25_000));
    assert!(admit(project).is_ok());
}

#[test]
fn absence_at_any_required_output_frame_still_rejects_export() {
    let (_root, project) = take(&[(0, Some(1)), (90_000, None), (110_000, Some(1))]);
    assert!(
        admit(project)
            .unwrap_err()
            .contains("required output frame")
    );
}

#[test]
fn runtime_id_reuse_never_rebinds_a_legacy_unpinned_subject_during_export() {
    let (_root, mut project) = take(&[(0, Some(1)), (25_000, None), (75_000, Some(2))]);
    project.shots[0].target_lifetime = None;
    assert!(
        admit(project)
            .unwrap_err()
            .contains("required output frame")
    );
}

#[test]
fn canonical_lifetime_survives_separate_checked_spans_and_retimed_source_clips() {
    let (_root, mut project) = take(&[(0, Some(1)), (150_000, Some(2))]);
    project.shots[0].target_lifetime = None;
    project.clips[0].source_range.end = MediaTime(100_000);
    let mut next = project.clips[0].clone();
    next.id = Id(4);
    next.at = MediaTime(100_000);
    next.source_range = interval(200_000, 400_000);
    project.clips.push(next);
    assert!(
        admit(project)
            .unwrap_err()
            .contains("required output frame")
    );
}

#[test]
fn rational_output_samples_map_to_exact_retimed_source_microseconds() {
    let (_root, mut project) = take(&[(0, Some(1)), (40_000, None), (50_000, Some(1))]);
    project.export.fps = FrameRate {
        numerator: 30_000,
        denominator: 1001,
    };
    project.clips[0].rate = PlaybackRate {
        numerator: 2,
        denominator: 1,
    };
    project.shots[0].span.end = project.clips[0].span().unwrap().end;
    let range = FrameRange::new(project.export.fps, project.duration(), None).unwrap();
    let map = project.clips[0].time_map().unwrap();
    let first: Vec<_> = source_samples(range, &map, project.sources[0].duration)
        .take(3)
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(first, [MediaTime(0), MediaTime(66_732), MediaTime(133_466)]);
    assert!(admit(project).is_ok());
    let (_root, mut project) = take(&[(0, Some(1)), (66_732, None), (66_733, Some(1))]);
    project.export.fps = FrameRate {
        numerator: 30_000,
        denominator: 1001,
    };
    project.clips[0].rate = PlaybackRate {
        numerator: 2,
        denominator: 1,
    };
    project.shots[0].span.end = project.clips[0].span().unwrap().end;
    assert!(
        admit(project)
            .unwrap_err()
            .contains("required output frame")
    );
}

/// A nonuniform source clock exercises the same analytic ramp used by actual rendered frames.
fn ramp(project: &mut Project) {
    project.clips[0].speed_ramp = Some(SpeedRamp {
        keys: vec![
            SpeedKey {
                at: MediaTime::ZERO,
                rate: PlaybackRate {
                    numerator: 1,
                    denominator: 2,
                },
                easing: Easing::Linear,
            },
            SpeedKey {
                at: MediaTime(300_000),
                rate: PlaybackRate {
                    numerator: 2,
                    denominator: 1,
                },
                easing: Easing::Smooth,
            },
        ],
    });
    project.shots[0].span.end = project.clips[0].span().unwrap().end;
}

#[test]
fn ramped_output_admission_matches_the_playback_owners_required_source_samples() {
    let (_root, mut project) = take(&[(0, Some(1))]);
    ramp(&mut project);
    let timeline = CompiledTimeline::new(Arc::new(project.clone())).unwrap();
    let mut plan = FramePlan::default();
    let frames = FrameRange::new(project.export.fps, project.duration(), None).unwrap();
    let map = project.clips[0].time_map().unwrap();
    for (index, source) in source_samples(frames, &map, project.sources[0].duration).enumerate() {
        timeline
            .sample(frames.time(index as u64).unwrap(), &mut plan, |_, _, _| {
                None
            })
            .unwrap();
        assert_eq!(source.unwrap(), plan.visuals[0].time);
    }
    timeline
        .sample(
            project.export.fps.frame_time(1).unwrap(),
            &mut plan,
            |_, _, _| None,
        )
        .unwrap();
    let required = plan.visuals[0].time;
    let (_root, mut accepted) = take(&[
        (0, Some(1)),
        (required.0 / 3, None),
        (required.0 * 2 / 3, Some(1)),
    ]);
    ramp(&mut accepted);
    assert!(admit(accepted).is_ok());
    let (_root, mut rejected) =
        take(&[(0, Some(1)), (required.0, None), (required.0 + 1, Some(1))]);
    ramp(&mut rejected);
    assert!(
        admit(rejected)
            .unwrap_err()
            .contains("required output frame")
    );
}

#[test]
fn native_dissolve_handles_are_checked_on_their_actual_required_output_frames() {
    let (_root, mut project) = take(&[(0, Some(1)), (190_000, None), (210_000, Some(1))]);
    project.clips[0].source_range.end = MediaTime(200_000);
    let mut next = project.clips[0].clone();
    next.id = Id(4);
    next.at = MediaTime(200_000);
    next.source_range = interval(400_000, 600_000);
    project.clips.push(next);
    project.shots[0].span = interval(200_000, 300_000);
    project.transitions.push(Transition {
        id: Id(5),
        from: Id(2),
        to: Id(4),
        duration: MediaTime(100_000),
        kind: TransitionKind::Crossfade,
    });
    project.export.range = Some(interval(200_000, 300_000));
    assert!(
        admit(project)
            .unwrap_err()
            .contains("required output frame")
    );
}

#[test]
fn selected_eye_admission_matches_actual_pose_interpolation_at_the_output_sample() {
    let mut missing_eye = subject(1);
    missing_eye.pose.eye_offset = None;
    let (_root, mut project) = take_subjects([
        (0, vec![subject(1)]),
        (110_000, vec![missing_eye]),
        (200_000, vec![subject(1)]),
    ]);
    assert!(admit(project.clone()).is_ok());
    project.shots[0].mode = CameraMode::Player {
        actor: 7,
        fov: CameraPose::default().fov,
    };
    assert!(
        admit(project)
            .unwrap_err()
            .contains("required output frame")
    );
}

#[test]
fn sample_streams_accept_held_timestamps_and_reject_reverse_or_cancelled_work() {
    let (_root, project) = take(&[(0, Some(1))]);
    let directory = &project.sources[0].path;
    let reference = recording_studio::camera::CameraTargetRef {
        lifetime: None,
        ..subject(1).reference()
    };
    let valid = recording_scene::targets::verify_samples(
        directory,
        [Ok(MediaTime(0)), Ok(MediaTime(0)), Ok(MediaTime(100_000))],
        reference,
        false,
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(valid, Some(subject(1).reference()));
    let error = recording_scene::targets::verify_samples(
        directory,
        [Ok(MediaTime(100_000)), Ok(MediaTime(0))],
        reference,
        false,
        &Cancellation::default(),
    )
    .unwrap_err();
    assert!(error.contains("nondecreasing"));
    let cancellation = Cancellation::default();
    let times = [0, 100_000].into_iter().map(|at| {
        if at > 0 {
            cancellation.cancel();
        }
        Ok(MediaTime(at))
    });
    let error =
        recording_scene::targets::verify_samples(directory, times, reference, false, &cancellation)
            .unwrap_err();
    assert!(error.contains("cancelled"));
}
