use super::*;
use recording_studio::{
    MediaTime,
    camera::{ActorPose, CameraPose, CameraTargetKind},
};

/// Immutable recorded target facts are sufficient to exercise cache admission without media.
fn preview() -> CameraPreview {
    CameraPreview {
        world_labels: Default::default(),
        epoch: [1, 2, 3],
        source: Id(1),
        source_time: MediaTime::ZERO,
        pose: CameraPose::default(),
        render_pose: CameraPose::default(),
        interactive: true,
        viewport: [0.0, 0.0, 1.0, 1.0],
        local_actor: 0,
        targets: (0..10)
            .map(|id| CameraTarget {
                runtime_id: id,
                unique_id: id as i64,
                spawn_revision: 1,
                name: Arc::from(format!("Player {id}")),
                kind: CameraTargetKind::Player,
                entity_type: None,
                pose: ActorPose {
                    position: [0.0; 3],
                    velocity: [0.0; 3],
                    yaw: 0.0,
                    pitch: 0.0,
                    roll: 0.0,
                    eye_offset: None,
                },
                teleported: false,
            })
            .collect(),
    }
}

#[test]
fn cache_admission_is_bound_to_source_publication_query_filter_and_local_actor() {
    let original = preview();
    let search = CameraSubjectSearch::new("player").unwrap();
    let results = CameraSubjectResults::new(&original, &search, CameraSubjectFilter::Players);
    assert!(results.is_current(&original, &search, CameraSubjectFilter::Players));
    assert!(Arc::ptr_eq(&results.indices, &results.clone().indices));
    assert!(!results.is_current(
        &original,
        &CameraSubjectSearch::new("other").unwrap(),
        CameraSubjectFilter::Players
    ));
    assert!(!results.is_current(&original, &search, CameraSubjectFilter::All));
    let mut changed = original.clone();
    changed.source = Id(2);
    assert!(!results.is_current(&changed, &search, CameraSubjectFilter::Players));
    changed = original.clone();
    changed.local_actor = 9;
    assert!(!results.is_current(&changed, &search, CameraSubjectFilter::Players));
    changed = original.clone();
    changed.targets = original.targets.to_vec().into();
    assert!(!results.is_current(&changed, &search, CameraSubjectFilter::Players));
}

#[test]
fn bounded_pages_expose_every_captured_spawn_reference_without_name_based_identity() {
    let mut preview = preview();
    for target in Arc::make_mut(&mut preview.targets) {
        target.name = Arc::from("Same name");
    }
    let results = CameraSubjectResults::new(
        &preview,
        &CameraSubjectSearch::new("same").unwrap(),
        CameraSubjectFilter::Players,
    );
    let mut visited = std::collections::BTreeSet::new();
    for first in (0..results.len()).step_by(CAMERA_SUBJECT_ROWS) {
        let range = results.range(first);
        assert!(range.len() <= CAMERA_SUBJECT_ROWS);
        for index in range {
            let target = results.target(index).unwrap();
            assert_eq!(target.reference(), preview.targets[index].reference());
            visited.insert(target.unique_id);
        }
    }
    assert_eq!(visited.len(), preview.targets.len());
}
