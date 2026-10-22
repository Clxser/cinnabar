use super::*;
use crate::menu::studio::camera::authoring_tests::inspected_project;

#[test]
fn session_views_restore_exact_framing_without_editing_or_seeking_recorded_state() {
    let mut studio = inspected_project(false);
    let original = studio.view.camera_workspace.view.unwrap().pose;
    let revision = studio.editor.as_ref().unwrap().revision();
    let epoch = studio.transport.epoch();
    let dirty = studio.editor.as_ref().unwrap().dirty();
    studio.save_camera_bookmark().unwrap();
    studio.save_camera_bookmark().unwrap();
    assert_eq!(studio.view.camera_workspace.bookmarks.len(), 1);
    studio.inspect_camera_pose(CameraPose {
        position: [8.0, 67.0, -4.0],
        ..original
    });
    studio.recall_camera_bookmark(0).unwrap();
    assert_eq!(studio.view.camera_workspace.view.unwrap().pose, original);
    assert_eq!(studio.editor.as_ref().unwrap().revision(), revision);
    assert_eq!(studio.editor.as_ref().unwrap().dirty(), dirty);
    assert_eq!(studio.transport.epoch(), epoch);
}

#[test]
fn session_views_cannot_silently_apply_coordinates_to_another_recording() {
    let mut studio = inspected_project(false);
    studio.save_camera_bookmark().unwrap();
    Arc::make_mut(&mut studio.view)
        .preview
        .camera
        .as_mut()
        .unwrap()
        .source = Id(99);
    let before = studio.view.camera_workspace.view;
    assert!(studio.recall_camera_bookmark(0).is_err());
    assert_eq!(studio.view.camera_workspace.view, before);
}

#[test]
fn session_view_history_retains_the_latest_eight_distinct_positions() {
    let mut studio = inspected_project(false);
    for index in 0..20 {
        studio.inspect_camera_pose(CameraPose {
            position: [index as f64, 64.0, 4.0],
            ..Default::default()
        });
        studio.save_camera_bookmark().unwrap();
    }
    assert_eq!(
        studio.view.camera_workspace.bookmarks.len(),
        MAX_SESSION_VIEWS
    );
    assert_eq!(
        studio
            .view
            .camera_workspace
            .bookmarks
            .first()
            .unwrap()
            .pose
            .position[0],
        12.0
    );
    assert_eq!(
        studio
            .view
            .camera_workspace
            .bookmarks
            .last()
            .unwrap()
            .pose
            .position[0],
        19.0
    );
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
