use super::*;

/// A frozen ellipse converts screen positions into reproducible angular samples.
fn point(degrees: f64) -> [f32; 2] {
    let radians = degrees.to_radians();
    [
        (100.0 + 100.0 * radians.cos()) as f32,
        (50.0 + 50.0 * radians.sin()) as f32,
    ]
}

#[test]
fn yaw_wrap_and_full_turns_remain_continuous_without_changing_pose_components() {
    let pose = CameraPose {
        yaw: 359.0,
        roll: -25.0,
        ..Default::default()
    };
    let mut drag = CameraRotationDrag::new(
        pose,
        CameraRotationAxis::Yaw,
        [0, 0, 200, 100],
        point(170.0),
    )
    .unwrap();
    for (cursor, expected) in [
        (179.0, 368.0),
        (-179.0, 370.0),
        (-90.0, 459.0),
        (0.0, 549.0),
        (90.0, 639.0),
        (170.0, 719.0),
    ] {
        assert!((drag.sample(point(cursor)).unwrap() - expected).abs() < 1e-4);
    }
}

#[test]
fn pitch_stops_at_poles_and_reverses_immediately_roll_preserves_unwrapped_values() {
    let pose = CameraPose {
        pitch: 85.0,
        roll: 355.0,
        ..Default::default()
    };
    let mut pitch = CameraRotationDrag::new(
        pose,
        CameraRotationAxis::Pitch,
        [0, 0, 200, 100],
        point(0.0),
    )
    .unwrap();
    assert_eq!(pitch.sample(point(90.0)), Some(90.0));
    assert!((pitch.sample(point(80.0)).unwrap() - 80.0).abs() < 1e-4);
    let mut roll = CameraRotationDrag::new(
        pose,
        CameraRotationAxis::Roll,
        [0, 0, 200, 100],
        point(170.0),
    )
    .unwrap();
    assert!((roll.sample(point(-170.0)).unwrap() - 375.0).abs() < 1e-4);
    assert!(roll.sample([100.0, 50.0]).is_none());
    assert!(roll.sample([f32::NAN, 50.0]).is_none());
    assert!((roll.sample(point(-160.0)).unwrap() - 385.0).abs() < 1e-4);
}

#[test]
fn free_and_exact_global_path_rotations_preserve_geometry_lens_handles_and_easing() {
    let span = Span::new(MediaTime::ZERO, MediaTime(2_000_000)).unwrap();
    for kind in [CameraRigKind::Free, CameraRigKind::Path] {
        let mut shot =
            CameraShot::from_preview(Id(1), span, kind, CameraPose::default(), None).unwrap();
        shot = shot
            .with_fov_key(span.end, 54.0)
            .unwrap()
            .with_fov_easing(span.end, Easing::EaseIn)
            .unwrap();
        if kind == CameraRigKind::Path {
            shot = shot
                .with_path_control(span.start, PathControl::Outgoing, [2.0, 65.0, 3.0])
                .unwrap()
                .with_key_easing(span.start, Easing::Smooth)
                .unwrap()
                .with_path_timing(PathTiming::GlobalDistance)
                .unwrap();
        }
        let original_pose = shot.rotation_pose(span.start).unwrap();
        for axis in CameraRotationAxis::ALL {
            let rotated = shot.with_rotation(span.start, axis, 45.0).unwrap();
            let pose = rotated.rotation_pose(span.start).unwrap();
            assert_eq!(axis.value(pose), 45.0);
            assert_eq!(pose.position, original_pose.position);
            assert_eq!(pose.fov, original_pose.fov);
            for untouched in CameraRotationAxis::ALL
                .into_iter()
                .filter(|other| *other != axis)
            {
                assert_eq!(untouched.value(pose), untouched.value(original_pose));
            }
            assert_eq!(rotated.fov_curve, shot.fov_curve);
            assert_eq!(rotated.path_timing, shot.path_timing);
            if kind == CameraRigKind::Path {
                assert_eq!(
                    rotated.key_at(span.start).unwrap().handles,
                    shot.key_at(span.start).unwrap().handles
                );
                assert_eq!(
                    rotated.key_at(span.start).unwrap().easing,
                    shot.key_at(span.start).unwrap().easing
                );
                let old = CompiledCameraPath::new(&shot).unwrap();
                let new = CompiledCameraPath::new(&rotated).unwrap();
                for at in [span.start, MediaTime(1_000_000), span.end] {
                    assert_eq!(
                        old.sample(at).unwrap().position,
                        new.sample(at).unwrap().position
                    );
                }
                assert!(rotated.rotation_pose(MediaTime(500_000)).is_err());
            }
        }
        assert!(
            shot.with_rotation(span.start, CameraRotationAxis::Yaw, f64::NAN)
                .is_err()
        );
        assert!(shot.rotation_pose(MediaTime(span.end.0 + 1)).is_err());
    }
}

#[test]
fn frozen_ring_geometry_is_deterministic_and_rejects_invalid_hit_origins() {
    let mut drag = CameraRotationDrag::new(
        CameraPose::default(),
        CameraRotationAxis::Roll,
        [0, 0, 200, 100],
        point(0.0),
    )
    .unwrap();
    let mut duplicate = drag;
    for at in [20.0, 55.0, 95.0, 95.0, 30.0, 0.0] {
        assert_eq!(drag.sample(point(at)), duplicate.sample(point(at)));
    }
    assert!(
        CameraRotationDrag::new(
            CameraPose::default(),
            CameraRotationAxis::Yaw,
            [0; 4],
            point(0.0)
        )
        .is_none()
    );
    assert!(
        CameraRotationDrag::new(
            CameraPose::default(),
            CameraRotationAxis::Yaw,
            [0, 0, 200, 100],
            [100.0, 50.0]
        )
        .is_none()
    );
}
