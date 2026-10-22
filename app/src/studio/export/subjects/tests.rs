use super::*;
use recording_studio::{FrameRate, MediaTime};

mod samples;

/// Frame windows use the same rational grid as export rather than rounding the requested In point.
fn interval(start: u64, end: u64) -> Span {
    Span::new(MediaTime(start), MediaTime(end)).unwrap()
}

#[test]
fn subject_preflight_covers_only_actual_export_samples_inside_the_shot() {
    let fps = FrameRate {
        numerator: 30_000,
        denominator: 1001,
    };
    let range =
        FrameRange::new(fps, MediaTime(1_000_000), Some(interval(20_000, 150_000))).unwrap();
    let sampled = sampled_intersection(interval(0, 1_000_000), interval(40_000, 140_000), range)
        .unwrap()
        .unwrap();
    assert_eq!(sampled.start, fps.frame_time(2).unwrap());
    assert_eq!(
        sampled.end,
        fps.frame_time(4)
            .unwrap()
            .checked_add(MediaTime(1))
            .unwrap()
    );
    assert_eq!(
        sampled_intersection(interval(0, 1_000_000), interval(40_000, 50_000), range).unwrap(),
        None
    );
}

#[test]
fn exact_half_open_boundaries_exclude_the_first_frame_after_the_camera_shot() {
    let fps = FrameRate::default();
    let range = FrameRange::new(fps, MediaTime(1_000_000), None).unwrap();
    let end = fps.frame_time(3).unwrap();
    let sampled = sampled_intersection(
        interval(0, 1_000_000),
        Span::new(fps.frame_time(1).unwrap(), end).unwrap(),
        range,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        sampled.end,
        fps.frame_time(2)
            .unwrap()
            .checked_add(MediaTime(1))
            .unwrap()
    );
}

/// Overlapping authored cameras retain the playback owner's precedence, even if the hidden shot needs a subject.
fn camera_project() -> recording_studio::Project {
    use recording_studio::{
        Id, PlaybackRate, Project,
        camera::{CameraPose, CameraRigKind, CameraShot},
        model::{Clip, Framing, MediaSource},
    };
    let mut project = Project::new("Camera admission");
    project.sources.push(MediaSource {
        id: Id(1),
        name: "Native take".into(),
        path: "not-opened-by-preflight".into(),
        kind: MediaKind::Replay,
        duration: MediaTime(1_000_000),
        size: [16, 16],
        frame_rate: None,
        sha256: None,
    });
    project.clips.push(Clip {
        id: Id(2),
        source: Id(1),
        source_range: interval(0, 1_000_000),
        at: MediaTime::ZERO,
        rate: PlaybackRate::default(),
        speed_ramp: None,
        track: 0,
        framing: Framing::default(),
        enabled: true,
    });
    let mut hidden = CameraShot::from_preview(
        Id(3),
        interval(0, 1_000_000),
        CameraRigKind::Free,
        CameraPose::default(),
        None,
    )
    .unwrap();
    hidden.mode = CameraMode::Player {
        actor: 7,
        fov: CameraPose::default().fov,
    };
    let visible = CameraShot::from_preview(
        Id(4),
        interval(0, 1_000_000),
        CameraRigKind::Free,
        CameraPose::default(),
        None,
    )
    .unwrap();
    project.shots.extend([hidden, visible]);
    project
}

#[test]
fn superseded_and_muted_subject_cameras_do_not_open_or_reject_unused_sources() {
    let project = std::sync::Arc::new(camera_project());
    let cancellation = Cancellation::default();
    assert!(
        active_camera_ranges(&project, &cancellation)
            .unwrap()
            .iter()
            .all(|(id, _)| *id == recording_studio::Id(4))
    );
    let request = ExportRequest {
        project,
        source_base: Default::default(),
        destination: recording_studio::export::Destination::Directory(Default::default()),
    };
    assert!(prepare(&request, &cancellation).is_ok());
    let mut project = camera_project();
    project.shots.pop();
    project
        .muted_tracks
        .insert(recording_studio::tracks::Track::Camera);
    let request = ExportRequest {
        project: std::sync::Arc::new(project),
        ..request
    };
    assert!(
        active_camera_ranges(&request.project, &cancellation)
            .unwrap()
            .is_empty()
    );
    assert!(prepare(&request, &cancellation).is_ok());
}
