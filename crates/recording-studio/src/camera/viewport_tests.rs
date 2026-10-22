//! Projection and curve edits stay deterministic without a window, carrier or live world.

use super::*;

/// Uses a linear two-key rail so explicit handles have a directly measurable geometric effect.
fn rail() -> CameraShot {
    CameraShot {
        id: Id(1),
        name: "Rail".into(),
        span: Span::new(MediaTime::ZERO, MediaTime(1_000_000)).unwrap(),
        mode: CameraMode::Path {
            keys: vec![
                CameraKey {
                    at: MediaTime::ZERO,
                    pose: CameraPose {
                        position: [0.0; 3],
                        ..CameraPose::default()
                    },
                    easing: Easing::Linear,
                    handles: None,
                },
                CameraKey {
                    at: MediaTime(1_000_000),
                    pose: CameraPose {
                        position: [12.0, 0.0, 0.0],
                        ..CameraPose::default()
                    },
                    easing: Easing::Linear,
                    handles: None,
                },
            ],
            smooth_position: false,
        },
        hide_hud: true,
        hide_hand: true,
        target_lifetime: None,
        fov_curve: None,
        fov_offset: MediaTime::ZERO,
        path_timing: PathTiming::Keyed,
        path_offset: MediaTime::ZERO,
    }
}

#[test]
fn dragged_controls_follow_the_projected_pointer_delta_at_the_original_depth() {
    let projection = CameraProjection::new(
        CameraPose {
            roll: 23.0,
            fov: 90.0,
            ..CameraPose::default()
        },
        1.6,
    )
    .unwrap();
    let point = [-2.0, 65.0, 10.0];
    let before = projection.project(point).unwrap();
    let after = projection
        .project(projection.dragged(point, [0.125, -0.2]).unwrap())
        .unwrap();
    assert!((after[0] - before[0] - 0.125).abs() < 1e-10);
    assert!((after[1] - before[1] + 0.2).abs() < 1e-10);
    assert!(projection.project([0.0, 64.0, -1.0]).is_none());
    assert!(
        projection
            .segment([0.0, 64.0, -1.0], [0.0, 64.0, 10.0])
            .is_some()
    );
}

#[test]
fn world_axis_drags_and_grid_snaps_preserve_every_unselected_coordinate() {
    let pose = CameraPose {
        position: [-5.0, 66.0, -5.0],
        yaw: -35.0,
        pitch: 10.0,
        ..Default::default()
    };
    let projection = CameraProjection::new(pose, 1.6).unwrap();
    let point = [1.03, 64.07, 8.09];
    for axis in CameraDragAxis::WORLD {
        let next = projection
            .dragged_constrained(point, [0.12, -0.08], axis, CameraSnap::Half)
            .unwrap();
        let selected = axis.index().unwrap();
        for coordinate in 0..3 {
            if coordinate != selected {
                assert_eq!(next[coordinate], point[coordinate]);
            }
        }
        assert!((next[selected] * 2.0 - (next[selected] * 2.0).round()).abs() < 1e-10);
        assert_eq!(
            projection.dragged_constrained(point, [0.0; 2], axis, CameraSnap::Block),
            Some(point)
        );
    }
}

#[test]
fn edge_on_axis_drags_fail_without_large_or_nonfinite_coordinate_jumps() {
    let projection = CameraProjection::new(
        CameraPose {
            position: [0.0; 3],
            ..Default::default()
        },
        1.0,
    )
    .unwrap();
    assert!(
        projection
            .dragged_constrained(
                [0.0, 0.0, 8.0],
                [0.0, 0.0],
                CameraDragAxis::Z,
                CameraSnap::Off
            )
            .is_some()
    );
    assert!(
        projection
            .dragged_constrained(
                [0.0, 0.0, 8.0],
                [f64::NAN, 0.0],
                CameraDragAxis::Z,
                CameraSnap::Off
            )
            .is_none()
    );
    let framed = CameraPose::default()
        .framed([0.0, 65.0, 0.0], 1.35)
        .unwrap();
    framed.validate().unwrap();
    assert_eq!(framed.roll, CameraPose::default().roll);
}

#[test]
fn projection_matches_bedrock_forward_and_rolled_screen_right() {
    let pose = CameraPose {
        fov: 90.0,
        ..CameraPose::default()
    };
    let front = CameraProjection::new(pose, 1.0)
        .unwrap()
        .project([-2.0, 64.0, 4.0])
        .unwrap();
    assert!((front[0] - 0.75).abs() < 1e-10);
    assert!((front[1] - 0.5).abs() < 1e-10);
    let turned = CameraProjection::new(CameraPose { yaw: 90.0, ..pose }, 1.0)
        .unwrap()
        .project([-4.0, 64.0, -2.0])
        .unwrap();
    assert!((turned[0] - 0.75).abs() < 1e-10);
    let rolled = CameraProjection::new(CameraPose { roll: 90.0, ..pose }, 1.0)
        .unwrap()
        .project([0.0, 66.0, 4.0])
        .unwrap();
    assert!((rolled[0] - 0.75).abs() < 1e-10);
    let looking_right = pose.flown([0.0; 3], [10.0, 0.0], 0.0, 8.0).unwrap();
    assert!(
        CameraProjection::new(looking_right, 1.0)
            .unwrap()
            .project([0.0, 64.0, 4.0])
            .unwrap()[0]
            < 0.5
    );
}

#[test]
fn authored_handles_change_the_evaluated_curve_and_survive_pose_edits() {
    let rail = rail();
    let curved = rail
        .with_path_control(MediaTime::ZERO, PathControl::Outgoing, [4.0, 8.0, 0.0])
        .unwrap();
    assert_eq!(
        rail.path_pose(MediaTime(500_000)).unwrap().position,
        [6.0, 0.0, 0.0]
    );
    assert_eq!(
        curved.path_pose(MediaTime(500_000)).unwrap().position,
        [6.0, 3.0, 0.0]
    );
    let moved = curved
        .with_pose(
            MediaTime::ZERO,
            CameraPose {
                position: [1.0, 2.0, 3.0],
                ..CameraPose::default()
            },
        )
        .unwrap();
    assert_eq!(
        moved.path_control(MediaTime::ZERO, PathControl::Outgoing),
        Some([5.0, 10.0, 3.0])
    );
    assert_eq!(curved.trimmed(curved.span).unwrap(), curved);
    assert!(
        curved
            .with_path_control(MediaTime::ZERO, PathControl::Outgoing, [f64::NAN, 0.0, 0.0])
            .is_err()
    );
}

#[test]
fn legacy_camera_keys_keep_automatic_handles_and_navigation_has_a_speed_bound() {
    let key: CameraKey = serde_json::from_value(serde_json::json!({
        "at": 0, "pose": { "position": [0.0, 64.0, 0.0], "yaw": 0.0, "pitch": 0.0, "roll": 0.0, "fov": 70.0 }, "easing": "linear"
    })).unwrap();
    assert_eq!(key.handles, None);
    let pose = CameraPose::default();
    let next = pose.flown([1.0; 3], [0.0; 2], 1.0, 8.0).unwrap();
    let distance = DVec3::from_array(next.position).distance(DVec3::from_array(pose.position));
    assert!(distance <= 0.8 + 1e-10);
    assert!(
        pose.flown([f64::NAN, 0.0, 0.0], [0.0; 2], 0.1, 8.0)
            .is_none()
    );
}

#[test]
fn orbit_drag_preserves_radius_and_framing_without_creating_a_zero_delta_jump() {
    let pose = CameraPose {
        position: [2.0, 67.0, -8.0],
        yaw: 12.0,
        pitch: 8.0,
        roll: 7.0,
        ..Default::default()
    };
    let pivot = pose.orbit_anchor(8.0).unwrap();
    assert_eq!(pose.orbited(pivot, [0.0, 0.0]), Some(pose));
    let rotated = pose.orbited(pivot, [90.0, 20.0]).unwrap();
    let radius = |position: [f64; 3]| {
        position
            .into_iter()
            .zip(pivot)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt()
    };
    assert!((radius(pose.position) - radius(rotated.position)).abs() < 1e-9);
    assert_eq!(rotated.roll, pose.roll);
    assert_eq!(rotated.fov, pose.fov);
    assert!(rotated.pitch > pose.pitch);
    assert!(pose.orbited(pivot, [f64::NAN, 0.0]).is_none());
}
