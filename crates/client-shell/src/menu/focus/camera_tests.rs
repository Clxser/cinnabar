use super::*;
use recording_studio::camera::{CameraPose, CameraView};
use std::sync::Arc;

/// Models the same focus callbacks used by Tab and gamepad directional navigation.
fn focused_viewport() -> MenuRuntime {
    let mut menu = MenuRuntime::new(true, 2, "Camera".into());
    menu.screen = MenuScreen::RecordingStudio;
    let workspace = &mut Arc::make_mut(&mut menu.studio.view).camera_workspace;
    workspace.focused = true;
    workspace.looking = true;
    workspace.view = Some(CameraView {
        pose: CameraPose::default(),
        revision: 1,
    });
    menu
}

#[test]
fn tab_focus_releases_fly_input_without_discarding_the_inspection_pose() {
    let mut menu = focused_viewport();
    let pose = menu.studio.view.camera_workspace.view;
    menu.move_focus(1);
    assert!(!menu.studio.view.camera_workspace.focused);
    assert!(!menu.studio.view.camera_workspace.looking);
    assert_eq!(menu.studio.view.camera_workspace.view, pose);
}

#[test]
fn directional_gamepad_focus_releases_fly_input() {
    for axis in [SettingsFocusAxis::Horizontal, SettingsFocusAxis::Vertical] {
        let mut menu = focused_viewport();
        menu.move_directional_focus(axis, 1);
        assert!(!menu.studio.view.camera_workspace.focused);
        assert!(!menu.studio.view.camera_workspace.looking);
    }
}
