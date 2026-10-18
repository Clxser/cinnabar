use super::*;
use recording_studio::{
    camera::{CameraPose, CameraView},
    effect::{Curve, EffectClip, EffectKind},
    playback::{CompiledTimeline, FramePlan},
};

/// Admits only metadata: camera authoring needs no GPU, recording files or filesystem workers.
pub(super) fn inspected_project(path: bool) -> Studio {
    let mut project = Project::new("Camera authoring");
    let span = Span::new(MediaTime::ZERO, MediaTime(2_000_000)).unwrap();
    project.sources.push(MediaSource {
        id: Id(1),
        name: "Replay".into(),
        path: "replay".into(),
        kind: MediaKind::Replay,
        duration: span.end,
        size: [1280, 720],
        frame_rate: None,
        sha256: None,
    });
    project.clips.push(Clip {
        id: Id(2),
        source: Id(1),
        source_range: span,
        at: MediaTime::ZERO,
        rate: PlaybackRate::default(),
        speed_ramp: None,
        track: 0,
        framing: Framing::default(),
        enabled: true,
    });
    if path {
        project.shots.push(
            CameraShot::from_preview(
                Id(3),
                span,
                CameraRigKind::Path,
                CameraPose::default(),
                None,
            )
            .unwrap(),
        );
    }
    for (id, kind, value) in [
        (Id(4), EffectKind::Zoom, 10.0),
        (Id(5), EffectKind::Shake, 2.0),
    ] {
        project.effects.push(EffectClip {
            id,
            span,
            kind,
            amount: Curve::constant(value),
            enabled: true,
            origin: None,
        });
    }
    let mut studio = Studio::new(PathBuf::new());
    studio.install(Editor::new(project).unwrap(), None);
    let at = MediaTime(500_000);
    studio.transport.seek(at, span.end);
    let epoch = [
        studio.document,
        studio.editor.as_ref().unwrap().revision(),
        studio.transport.epoch(),
    ];
    let inspected = CameraPose {
        position: [5.0, 70.0, 8.0],
        yaw: 90.0,
        pitch: 15.0,
        fov: 60.0,
        ..CameraPose::default()
    };
    let view = Arc::make_mut(&mut studio.view);
    view.selected = path.then_some(Id(3));
    view.camera_workspace.view = Some(CameraView {
        pose: inspected,
        revision: 1,
    });
    view.preview = recording_studio::preview::PreviewStatus {
        ready: true,
        at,
        epoch: Some(epoch),
        camera: Some(CameraPreview {
            world_labels: Default::default(),
            epoch,
            source: Id(1),
            source_time: at,
            pose: CameraPose::default(),
            render_pose: inspected,
            interactive: true,
            viewport: [0.0, 0.0, 1.0, 1.0],
            local_actor: 0,
            targets: Arc::from([]),
        }),
        ..Default::default()
    };
    studio
}

/// Evaluates the saved project with its effects, independent of the inspection override.
fn authored_output(studio: &Studio) -> CameraPose {
    let timeline = CompiledTimeline::new(Arc::clone(studio.project().unwrap())).unwrap();
    let mut plan = FramePlan::default();
    timeline
        .sample(MediaTime(500_000), &mut plan, |_, _, _| None)
        .unwrap();
    plan.camera.unwrap()
}

/// Floating point inverse effects preserve the displayed pose within their arithmetic precision.
fn assert_pose(actual: CameraPose, expected: CameraPose) {
    assert_eq!(actual.position, expected.position);
    for (actual, expected) in [actual.yaw, actual.pitch, actual.roll, actual.fov]
        .into_iter()
        .zip([expected.yaw, expected.pitch, expected.roll, expected.fov])
    {
        assert!((actual - expected).abs() < 1e-10);
    }
}

#[test]
fn adding_free_or_path_cameras_authors_the_inspection_pose_beneath_effects() {
    for kind in [CameraRigKind::Free, CameraRigKind::Path] {
        let mut studio = inspected_project(false);
        let inspected = studio.view.camera_workspace.view.unwrap().pose;
        studio.create_camera(kind).unwrap();
        assert_pose(authored_output(&studio), inspected);
        assert!(studio.view.camera_workspace.view.is_none());
    }
}

#[test]
fn adding_a_path_key_authors_the_inspection_pose_beneath_effects() {
    let mut studio = inspected_project(true);
    let inspected = studio.view.camera_workspace.view.unwrap().pose;
    studio.add_camera_key().unwrap();
    assert_pose(authored_output(&studio), inspected);
}

#[test]
fn ordinary_creation_keeps_the_admitted_unmodified_camera_pose() {
    let mut studio = inspected_project(false);
    Arc::make_mut(&mut studio.view).camera_workspace.view = None;
    studio.create_camera(CameraRigKind::Free).unwrap();
    assert_eq!(studio.project().unwrap().shots[0].path_keys().len(), 0);
    assert_eq!(
        studio.project().unwrap().shots[0].sample(MediaTime(500_000), |_| None),
        Some(CameraPose::default())
    );
}

#[test]
fn independent_lens_controls_commit_one_undoable_edit_and_keep_the_path_intact() {
    let mut studio = inspected_project(true);
    let before = studio.project().unwrap().shots[0].clone();
    studio.camera_fov_action(Action::AddCameraFovKey).unwrap();
    let after = studio.project().unwrap().shots[0].clone();
    assert_eq!(before.mode, after.mode);
    assert!(after.fov_key_at(studio.transport.position()).is_some());
    studio
        .camera_fov_action(Action::CycleCameraFovEasing)
        .unwrap();
    assert_ne!(
        after.fov_curve,
        studio.project().unwrap().shots[0].fov_curve
    );
    studio.editor.as_mut().unwrap().undo().unwrap();
    assert_eq!(studio.project().unwrap().shots[0], after);
    studio.editor.as_mut().unwrap().undo().unwrap();
    assert_eq!(studio.project().unwrap().shots[0], before);
}

#[test]
fn dragging_the_view_orbits_transiently_without_editing_the_timeline() {
    let mut studio = inspected_project(false);
    let project = Arc::clone(studio.project().unwrap());
    let revision = studio.editor.as_ref().unwrap().revision();
    let initial = studio.view.camera_workspace.view.unwrap();
    let action = Some(launcher::menu::MenuAction::Studio(Action::CameraViewport {
        bounds: [0, 0, 800, 450],
    }));
    assert!(studio.camera_pointer(action, Some([400.0, 225.0]), true, true));
    assert!(studio.camera_pointer(None, Some([440.0, 240.0]), true, false));
    let rotated = studio.view.camera_workspace.view.unwrap();
    assert_ne!(rotated.pose.position, initial.pose.position);
    assert_ne!(rotated.pose.yaw, initial.pose.yaw);
    assert!(studio.camera_pointer(None, Some([440.0, 240.0]), true, false));
    assert_eq!(studio.view.camera_workspace.view, Some(rotated));
    assert!(studio.camera_pointer(None, Some([400.0, 225.0]), true, false));
    assert_eq!(
        studio.view.camera_workspace.view.unwrap().pose,
        initial.pose
    );
    assert!(studio.camera_pointer(None, Some([400.0, 225.0]), false, false));
    assert!(Arc::ptr_eq(studio.project().unwrap(), &project));
    assert_eq!(studio.editor.as_ref().unwrap().revision(), revision);
}
