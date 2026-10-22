use super::*;
use crate::menu::studio::camera::authoring_tests::inspected_project;
use recording_studio::camera::{CameraPose, CameraRigKind, CameraRotationAxis, PathTiming};

/// Anonymous native preview metadata admits authoring without files, workers or a GPU.
fn studio(path: bool) -> Studio {
    let mut studio = inspected_project(true);
    if !path {
        let span = studio.project().unwrap().shots[0].span;
        let shot = CameraShot::from_preview(
            Id(3),
            span,
            CameraRigKind::Free,
            CameraPose::default(),
            None,
        )
        .unwrap();
        studio
            .edit("Free camera", &[Command::ReplaceShot { shot }])
            .unwrap();
    }
    studio.transport.seek(MediaTime::ZERO, MediaTime(2_000_000));
    let epoch = [
        studio.document,
        studio.editor.as_ref().unwrap().revision(),
        studio.transport.epoch(),
    ];
    let view = Arc::make_mut(&mut studio.view);
    view.selected = Some(Id(3));
    view.workspace.viewport_tool = ViewportTool::Rotate;
    view.preview.ready = true;
    view.preview.epoch = Some(epoch);
    view.preview.camera.as_mut().unwrap().epoch = epoch;
    studio
}

/// Each target uses its frozen painted ellipse, independent of current image or pointer hover.
fn action(axis: CameraRotationAxis, bounds: [i32; 4]) -> Option<MenuAction> {
    Some(MenuAction::Studio(Action::CameraRotationControl {
        shot: Id(3),
        at: MediaTime::ZERO,
        axis,
        bounds,
    }))
}

#[test]
fn free_and_global_path_rotation_commit_once_preserve_rail_and_undo_redo() {
    for path in [false, true] {
        for axis in CameraRotationAxis::ALL {
            let mut studio = studio(path);
            if path {
                let shot = studio.project().unwrap().shots[0]
                    .with_path_timing(PathTiming::GlobalDistance)
                    .unwrap();
                studio
                    .edit("Rail timing", &[Command::ReplaceShot { shot }])
                    .unwrap();
                let revision = studio.editor.as_ref().unwrap().revision();
                let view = Arc::make_mut(&mut studio.view);
                view.preview.camera.as_mut().unwrap().epoch[1] = revision;
                view.preview.ready = true;
            }
            let original = studio.project().unwrap().shots[0].clone();
            let revision = studio.editor.as_ref().unwrap().revision();
            assert!(studio.camera_pointer(
                action(axis, [0, 0, 200, 100]),
                Some([200.0, 50.0]),
                true,
                true
            ));
            assert!(studio.camera_pointer(None, Some([100.0, 100.0]), true, false));
            assert_eq!(studio.project().unwrap().shots[0], original);
            let expected = studio
                .view
                .camera_workspace
                .draft
                .as_ref()
                .unwrap()
                .as_ref()
                .clone();
            assert!(
                (axis.value(expected.rotation_pose(MediaTime::ZERO).unwrap()) - 90.0).abs() < 1e-6
            );
            assert_eq!(studio.editor.as_ref().unwrap().revision(), revision);
            assert!(studio.camera_pointer(None, Some([100.0, 100.0]), false, false));
            assert_eq!(studio.editor.as_ref().unwrap().revision(), revision + 1);
            assert_eq!(studio.project().unwrap().shots[0], expected);
            assert!(studio.view.camera_workspace.draft.is_none());
            assert!(studio.view.camera_workspace.rotation.is_none());
            assert!(studio.view.camera_workspace.view.is_none());
            studio.editor.as_mut().unwrap().undo().unwrap();
            assert_eq!(studio.project().unwrap().shots[0], original);
            studio.editor.as_mut().unwrap().redo().unwrap();
            assert_eq!(studio.project().unwrap().shots[0], expected);
        }
    }
}

#[test]
fn modal_locked_or_pending_image_cannot_start_an_authored_rotation() {
    for rejected in 0..5 {
        let mut studio = studio(false);
        let original = Arc::clone(studio.project().unwrap());
        let inspection = studio.view.camera_workspace.view;
        let view = Arc::make_mut(&mut studio.view);
        match rejected {
            0 => view.workflow.recovery.visible = true,
            1 => view.workflow.recovery.busy = true,
            2 => {
                view.workspace
                    .timeline
                    .locked
                    .insert(recording_studio::tracks::Track::Camera);
            }
            3 => view.preview.ready = false,
            _ => view.workspace.viewport_tool = ViewportTool::Navigate,
        }
        studio.camera_pointer(
            action(CameraRotationAxis::Yaw, [0, 0, 200, 100]),
            Some([200.0, 50.0]),
            true,
            true,
        );
        assert!(!studio.authored_camera_drag_active());
        assert!(Arc::ptr_eq(studio.project().unwrap(), &original));
        assert_eq!(studio.view.camera_workspace.view, inspection);
        assert!(studio.view.error.is_some());
    }
}

#[test]
fn current_source_preview_refresh_retains_pending_rotation_until_original_release() {
    let mut studio = studio(false);
    let revision = studio.editor.as_ref().unwrap().revision();
    studio.camera_pointer(
        action(CameraRotationAxis::Roll, [0, 0, 200, 100]),
        Some([200.0, 50.0]),
        true,
        true,
    );
    studio.camera_pointer(None, Some([100.0, 100.0]), true, false);
    Arc::make_mut(&mut studio.view).preview.ready = false;
    studio.camera_pointer(None, Some([0.0, 50.0]), true, false);
    assert!(studio.authored_camera_drag_active());
    assert!(
        (studio
            .view
            .camera_workspace
            .draft
            .as_ref()
            .unwrap()
            .rotation_pose(MediaTime::ZERO)
            .unwrap()
            .roll
            - 180.0)
            .abs()
            < 1e-6
    );
    studio.camera_pointer(None, Some([0.0, 50.0]), false, false);
    assert_eq!(studio.editor.as_ref().unwrap().revision(), revision + 1);
}

#[test]
fn escape_and_stationary_click_restore_inspection_without_edits() {
    for moved in [false, true] {
        let mut studio = studio(false);
        let original = Arc::clone(studio.project().unwrap());
        let inspection = studio.view.camera_workspace.view;
        let revision = studio.editor.as_ref().unwrap().revision();
        studio.camera_pointer(
            action(CameraRotationAxis::Roll, [0, 0, 200, 100]),
            Some([200.0, 50.0]),
            true,
            true,
        );
        if moved {
            studio.camera_pointer(None, Some([100.0, 100.0]), true, false);
            studio.release_camera();
        } else {
            studio.camera_pointer(None, Some([200.0, 50.0]), false, false);
        }
        assert!(Arc::ptr_eq(studio.project().unwrap(), &original));
        assert_eq!(studio.editor.as_ref().unwrap().revision(), revision);
        assert_eq!(studio.view.camera_workspace.view, inspection);
        assert!(studio.view.camera_workspace.draft.is_none());
        assert!(!studio.authored_camera_drag_active());
    }
}

#[test]
fn navigation_and_new_hover_geometry_cannot_steal_a_captured_rotation() {
    let mut studio = studio(false);
    studio.camera_pointer(
        action(CameraRotationAxis::Yaw, [0, 0, 200, 100]),
        Some([200.0, 50.0]),
        true,
        true,
    );
    studio.camera_pointer(None, Some([100.0, 100.0]), true, false);
    let draft = Arc::clone(studio.view.camera_workspace.draft.as_ref().unwrap());
    let frozen = studio.view.camera_workspace.rotation;
    assert!(!studio.pan_camera(Some([100.0, 100.0]), true, true, Some([0, 0, 800, 450])));
    studio.focus_camera().unwrap();
    studio.navigate_camera([1.0; 3], [25.0; 2], 0.1, true, true);
    assert!(studio.view.camera_workspace.view.is_none());
    studio.camera_pointer(
        action(CameraRotationAxis::Roll, [0, 0, 800, 450]),
        Some([100.0, 100.0]),
        true,
        true,
    );
    assert_eq!(studio.view.camera_workspace.rotation, frozen);
    assert!(Arc::ptr_eq(
        studio.view.camera_workspace.draft.as_ref().unwrap(),
        &draft
    ));
    studio.camera_pointer(None, Some([0.0, 50.0]), true, false);
    assert!(
        (studio
            .view
            .camera_workspace
            .draft
            .as_ref()
            .unwrap()
            .rotation_pose(MediaTime::ZERO)
            .unwrap()
            .yaw
            - 180.0)
            .abs()
            < 1e-6
    );
}

#[test]
fn changed_take_seek_document_or_recovery_cancels_and_restores_inspection() {
    for changed in 0..4 {
        let mut studio = studio(false);
        let inspection = studio.view.camera_workspace.view;
        let original = studio.project().unwrap().shots[0].clone();
        studio.camera_pointer(
            action(CameraRotationAxis::Pitch, [0, 0, 200, 100]),
            Some([200.0, 50.0]),
            true,
            true,
        );
        studio.camera_pointer(None, Some([100.0, 100.0]), true, false);
        match changed {
            0 => {
                Arc::make_mut(&mut studio.view)
                    .preview
                    .camera
                    .as_mut()
                    .unwrap()
                    .source = Id(999)
            }
            1 => studio
                .transport
                .seek(MediaTime(500_000), MediaTime(2_000_000)),
            2 => {
                studio
                    .editor
                    .as_mut()
                    .unwrap()
                    .apply(
                        "Other edit",
                        &[Command::Rename {
                            name: "Changed project".into(),
                        }],
                    )
                    .unwrap();
            }
            _ => Arc::make_mut(&mut studio.view).workflow.recovery.busy = true,
        }
        let revision = studio.editor.as_ref().unwrap().revision();
        studio.camera_pointer(None, Some([0.0, 50.0]), false, false);
        assert_eq!(studio.project().unwrap().shots[0], original);
        assert_eq!(studio.editor.as_ref().unwrap().revision(), revision);
        assert_eq!(studio.view.camera_workspace.view, inspection);
        assert!(!studio.authored_camera_drag_active());
        assert!(studio.camera_interaction.project.is_none());
    }
}
