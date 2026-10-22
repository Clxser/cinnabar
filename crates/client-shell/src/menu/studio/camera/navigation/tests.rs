use super::*;
use crate::menu::studio::camera::authoring_tests::inspected_project;

#[test]
fn middle_button_cannot_cancel_an_authored_path_draft_or_its_original_release() {
    use recording_studio::camera::PathControl;
    for handle in [PathControl::Key, PathControl::Outgoing] {
        let mut studio = inspected_project(true);
        let original = studio.project().unwrap().shots[0].clone();
        let revision = studio.editor.as_ref().unwrap().revision();
        let action = Some(launcher::menu::MenuAction::Studio(Action::CameraControl {
            shot: Id(3),
            at: MediaTime::ZERO,
            handle,
            bounds: [0, 0, 800, 450],
        }));
        assert!(studio.camera_pointer(action, Some([400.0, 225.0]), true, true));
        assert!(studio.camera_pointer(None, Some([440.0, 225.0]), true, false));
        let draft = Arc::clone(studio.view.camera_workspace.draft.as_ref().unwrap());
        assert!(studio.authored_camera_drag_active());
        assert!(!studio.pan_camera(Some([440.0, 225.0]), true, true, Some([0, 0, 800, 450])));
        assert!(Arc::ptr_eq(
            studio.view.camera_workspace.draft.as_ref().unwrap(),
            &draft
        ));
        assert!(studio.authored_camera_drag_active());
        assert!(studio.camera_pointer(None, Some([460.0, 225.0]), true, false));
        let expected = studio
            .view
            .camera_workspace
            .draft
            .as_ref()
            .unwrap()
            .as_ref()
            .clone();
        assert!(studio.camera_pointer(None, Some([460.0, 225.0]), false, false));
        assert_eq!(studio.editor.as_ref().unwrap().revision(), revision + 1);
        assert_eq!(studio.project().unwrap().shots[0], expected);
        studio.editor.as_mut().unwrap().undo().unwrap();
        assert_eq!(studio.project().unwrap().shots[0], original);
    }
}

#[test]
fn navigation_preferences_and_gestures_never_edit_or_seek_the_document() {
    let mut studio = inspected_project(true);
    let project = Arc::clone(studio.project().unwrap());
    let revision = studio.editor.as_ref().unwrap().revision();
    let epoch = studio.transport.epoch();
    let dirty = studio.view.dirty;
    let at = studio.transport.position();
    studio.focus_camera().unwrap();
    studio.camera_navigation_action(N::ToggleSmooth).unwrap();
    studio.camera_navigation_action(N::Speed(1)).unwrap();
    assert_eq!(
        studio.view.camera_workspace.navigation.speed(),
        recording_studio::camera::navigation::DEFAULT_SPEED * 2.0
    );
    studio.navigate_camera([1.0, 0.0, 0.0], [2.0, 0.0], 0.05, false, true);
    assert!(studio.camera_wheel(1.0, false, NavigationGesture::Zoom));
    let zoom = studio.view.camera_workspace.view.unwrap().pose;
    assert!(studio.pan_camera(Some([50.0, 50.0]), true, true, Some([0, 0, 200, 100])));
    assert!(studio.pan_camera(Some([80.0, 50.0]), true, false, Some([0, 0, 200, 100])));
    assert!(studio.pan_camera(None, false, false, None));
    assert_ne!(
        studio.view.camera_workspace.view.unwrap().pose.position,
        zoom.position
    );
    studio.camera_navigation_action(N::Level).unwrap();
    assert_eq!(studio.view.camera_workspace.view.unwrap().pose.roll, 0.0);
    assert!(Arc::ptr_eq(studio.project().unwrap(), &project));
    assert_eq!(studio.editor.as_ref().unwrap().revision(), revision);
    assert_eq!(studio.transport.epoch(), epoch);
    assert_eq!(studio.transport.position(), at);
    assert_eq!(studio.view.dirty, dirty);
}

#[test]
fn release_and_stale_seek_clear_momentum_without_moving_the_inspection_pose() {
    let mut studio = inspected_project(true);
    studio.focus_camera().unwrap();
    studio.camera_navigation_action(N::ToggleSmooth).unwrap();
    studio.navigate_camera([0.0, 0.0, 1.0], [0.0; 2], 0.1, false, false);
    let pose = studio.view.camera_workspace.view.unwrap().pose;
    studio.release_camera();
    assert_eq!(studio.camera_interaction.motion, Default::default());
    studio.navigate_camera([1.0; 3], [2.0; 2], 0.1, true, true);
    assert_eq!(studio.view.camera_workspace.view.unwrap().pose, pose);
    studio.focus_camera().unwrap();
    studio.navigate_camera([0.0; 3], [0.0; 2], 0.1, false, false);
    assert_eq!(studio.view.camera_workspace.view.unwrap().pose, pose);
    studio
        .transport
        .seek(MediaTime(750_000), MediaTime(2_000_000));
    studio.navigate_camera([1.0; 3], [0.0; 2], 0.1, true, false);
    assert!(!studio.view.camera_workspace.focused);
    assert_eq!(studio.camera_interaction.motion, Default::default());
    assert_eq!(studio.view.camera_workspace.view.unwrap().pose, pose);
}

#[test]
fn changing_take_rejects_an_active_pan_even_when_the_document_epoch_is_unchanged() {
    let mut studio = inspected_project(true);
    assert!(studio.pan_camera(Some([50.0; 2]), true, true, Some([0, 0, 100, 100])));
    let pose = studio.view.camera_workspace.view;
    Arc::make_mut(&mut studio.view)
        .preview
        .camera
        .as_mut()
        .unwrap()
        .source = Id(999);
    assert!(!studio.pan_camera(Some([75.0; 2]), true, false, Some([0, 0, 100, 100])));
    assert!(studio.camera_interaction.pan.is_none());
    assert_eq!(studio.view.camera_workspace.view, pose);
}

#[test]
fn view_only_preview_refresh_keeps_motion_but_level_discards_it_exactly() {
    let mut studio = inspected_project(true);
    studio.focus_camera().unwrap();
    studio.camera_navigation_action(N::ToggleSmooth).unwrap();
    studio.navigate_camera([0.0, 0.0, 1.0], [0.0; 2], 0.1, false, false);
    let first = studio.view.camera_workspace.view.unwrap().pose;
    Arc::make_mut(&mut studio.view).preview.ready = false;
    studio.navigate_camera([0.0; 3], [0.0; 2], 0.05, false, false);
    let coasted = studio.view.camera_workspace.view.unwrap().pose;
    assert!(coasted.position[2] > first.position[2]);
    studio.camera_navigation_action(N::Level).unwrap();
    assert_eq!(studio.camera_interaction.motion, Default::default());
    studio.navigate_camera([0.0; 3], [0.0; 2], 0.1, false, false);
    assert_eq!(
        studio.view.camera_workspace.view.unwrap().pose,
        coasted.leveled()
    );
}

#[test]
fn pixel_wheel_and_line_wheel_use_the_same_portable_motion_units() {
    for gesture in [
        NavigationGesture::Dolly,
        NavigationGesture::Zoom,
        NavigationGesture::Speed,
    ] {
        let mut line = inspected_project(true);
        let mut pixel = inspected_project(true);
        assert!(line.camera_wheel(1.0, false, gesture));
        assert!(pixel.camera_wheel(
            recording_studio::camera::navigation::WHEEL_PIXELS_PER_STEP as f32,
            true,
            gesture
        ));
        assert_eq!(
            line.view.camera_workspace.view.unwrap().pose,
            pixel.view.camera_workspace.view.unwrap().pose
        );
        assert_eq!(
            line.view.camera_workspace.navigation,
            pixel.view.camera_workspace.navigation
        );
    }
}

#[test]
fn global_rail_cache_survives_seeks_and_view_moves_but_invalidates_on_metadata_edits() {
    use recording_studio::camera::{PathContinuity, PathTiming};
    let mut studio = inspected_project(true);
    let shot = studio.camera_shot().unwrap();
    let shot = shot
        .keyed(
            shot.span.end,
            CameraPose {
                position: [10.0, 67.0, 12.0],
                ..Default::default()
            },
        )
        .unwrap()
        .with_path_timing(PathTiming::GlobalDistance)
        .unwrap();
    studio
        .edit("Prepare global rail", &[Command::ReplaceShot { shot }])
        .unwrap();
    studio.publish_editor();
    let first = Arc::clone(
        &studio
            .view
            .camera_workspace
            .path_cache
            .as_ref()
            .unwrap()
            .path,
    );
    studio
        .transport
        .seek(MediaTime(750_000), MediaTime(2_000_000));
    studio.publish_editor();
    assert!(Arc::ptr_eq(
        &first,
        &studio
            .view
            .camera_workspace
            .path_cache
            .as_ref()
            .unwrap()
            .path
    ));
    studio.inspect_camera_pose(CameraPose {
        position: [-2.0, 66.0, -4.0],
        ..Default::default()
    });
    studio.publish_editor();
    assert!(Arc::ptr_eq(
        &first,
        &studio
            .view
            .camera_workspace
            .path_cache
            .as_ref()
            .unwrap()
            .path
    ));
    let shot = studio
        .camera_shot()
        .unwrap()
        .with_path_continuity(PathContinuity::Smooth)
        .unwrap();
    studio
        .edit("Align rail tangents", &[Command::ReplaceShot { shot }])
        .unwrap();
    studio.publish_editor();
    assert!(!Arc::ptr_eq(
        &first,
        &studio
            .view
            .camera_workspace
            .path_cache
            .as_ref()
            .unwrap()
            .path
    ));
    assert_eq!(
        studio.project().unwrap().shots[0].path_continuity,
        PathContinuity::Smooth
    );
    studio.activate(Action::Undo);
    assert_eq!(
        studio.project().unwrap().shots[0].path_continuity,
        PathContinuity::Authored
    );
}
