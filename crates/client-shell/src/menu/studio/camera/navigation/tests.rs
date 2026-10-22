use super::*;
use crate::menu::studio::camera::authoring_tests::inspected_project;

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
