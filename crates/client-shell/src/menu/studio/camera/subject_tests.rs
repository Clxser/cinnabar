use super::*;
use recording_studio::camera::{ActorPose, CameraSubjectFilter, CameraTarget, CameraTargetKind};

/// Only class and the take's explicit local ID can establish default player eligibility.
fn target(id: u64, kind: CameraTargetKind) -> CameraTarget {
    CameraTarget {
        runtime_id: id,
        unique_id: id as i64,
        spawn_revision: 1,
        name: Arc::from("Player"),
        kind,
        entity_type: None,
        view: Default::default(),
        pose: ActorPose {
            position: [0.0; 3],
            velocity: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            eye_offset: None,
        },
        teleported: false,
    }
}

#[test]
fn default_target_cycles_and_clicks_exclude_entities_and_unclassified_remote_names() {
    let mut studio = authoring_tests::inspected_project(false);
    let entity = target(1, CameraTargetKind::Entity);
    let player = target(2, CameraTargetKind::Player);
    let unknown = target(3, CameraTargetKind::Unknown);
    let local = target(4, CameraTargetKind::Unknown);
    let camera = Arc::make_mut(&mut studio.view)
        .preview
        .camera
        .as_mut()
        .unwrap();
    camera.local_actor = local.runtime_id;
    camera.targets = Arc::from([entity.clone(), player.clone(), unknown, local.clone()]);
    studio.cycle_camera_target(1);
    assert_eq!(studio.view.camera_target, Some(player.reference()));
    studio.cycle_camera_target(1);
    assert_eq!(studio.view.camera_target, Some(local.reference()));
    assert!(studio.select_camera_subject(entity.reference()).is_err());
    studio.toggle_camera_subject_filter();
    assert_eq!(
        studio.view.camera_workspace.subject_filter,
        CameraSubjectFilter::All
    );
    assert!(studio.select_camera_subject(entity.reference()).is_ok());
    studio.toggle_camera_subject_filter();
    assert_eq!(studio.view.camera_target, None);
}
