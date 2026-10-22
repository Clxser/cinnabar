use super::*;

/// An asymmetric cubic exposes the speed variation of uniform Bezier parameter steps.
fn curved() -> CameraShot {
    let span = Span::new(MediaTime::ZERO, MediaTime(1_000_000)).unwrap();
    let mut shot = CameraShot::from_preview(
        Id(1),
        span,
        CameraRigKind::Path,
        CameraPose {
            position: [0.0; 3],
            yaw: 0.0,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    shot = shot
        .with_pose(
            span.end,
            CameraPose {
                position: [12.0, 0.0, 0.0],
                yaw: 90.0,
                ..Default::default()
            },
        )
        .unwrap();
    shot = shot
        .with_path_control(span.start, PathControl::Outgoing, [0.0, 12.0, 0.0])
        .unwrap();
    shot = shot
        .with_path_control(span.end, PathControl::Incoming, [4.0, 0.0, 0.0])
        .unwrap();
    shot.with_path_timing(PathTiming::Distance).unwrap()
}

#[test]
fn equal_time_steps_follow_nearly_equal_distance_around_an_asymmetric_curve() {
    let shot = curved();
    let path = CompiledCameraPath::new(&shot).unwrap();
    let mut previous = DVec3::from_array(path.sample(MediaTime::ZERO).unwrap().position);
    let mut lengths = Vec::new();
    for step in 1..=100 {
        let point = DVec3::from_array(path.sample(MediaTime(step * 10_000)).unwrap().position);
        lengths.push(point.distance(previous));
        previous = point;
    }
    let average = lengths.iter().sum::<f64>() / lengths.len() as f64;
    assert!(
        lengths
            .iter()
            .all(|distance| (distance / average - 1.0).abs() < 0.05)
    );
    assert_eq!(path.sample(shot.span.start).unwrap().position, [0.0; 3]);
    assert_eq!(
        path.sample(shot.span.end).unwrap().position,
        [12.0, 0.0, 0.0]
    );
}

#[test]
fn compiled_and_standalone_scrubs_keep_orientation_and_lens_easing_independent() {
    let mut shot = curved()
        .with_key_easing(MediaTime(1_000_000), Easing::EaseIn)
        .unwrap();
    shot = shot
        .with_fov_key(MediaTime::ZERO, 70.0)
        .unwrap()
        .with_fov_key(MediaTime(1_000_000), 40.0)
        .unwrap()
        .with_fov_easing(MediaTime(1_000_000), Easing::EaseOut)
        .unwrap();
    let path = CompiledCameraPath::new(&shot).unwrap();
    for at in [750_000, 250_000, 500_000, 125_000, 999_999] {
        let time = MediaTime(at);
        let expected = shot.sample(time, |_| None).unwrap();
        assert_eq!(
            shot.sample_compiled(time, |_| None, Some(&path)),
            Some(expected)
        );
        let amount = at as f64 / 1_000_000.0;
        assert!((expected.yaw - Easing::EaseIn.sample(amount) * 90.0).abs() < 1e-10);
        assert!((expected.fov - (70.0 - Easing::EaseOut.sample(amount) * 30.0)).abs() < 1e-10);
    }
}

#[test]
fn stationary_geometry_still_animates_orientation_without_invalid_arc_math() {
    let mut shot = curved();
    let CameraMode::Path { keys, .. } = &mut shot.mode else {
        unreachable!()
    };
    for key in keys {
        key.pose.position = [2.0, 64.0, 3.0];
        key.handles = None;
    }
    let path = CompiledCameraPath::new(&shot).unwrap();
    let pose = path.sample(MediaTime(500_000)).unwrap();
    assert_eq!(pose.position, [2.0, 64.0, 3.0]);
    pose.validate().unwrap();
    assert!(pose.yaw > 0.0 && pose.yaw < 90.0);
}

#[test]
fn legacy_json_keeps_keyed_motion_and_distance_metadata_round_trips() {
    let shot = curved();
    let mut value = serde_json::to_value(&shot).unwrap();
    value.as_object_mut().unwrap().remove("path_timing");
    let legacy: CameraShot = serde_json::from_value(value).unwrap();
    assert_eq!(legacy.path_timing, PathTiming::Keyed);
    assert!(CompiledCameraPath::new(&legacy).is_none());
    let decoded: CameraShot = serde_json::from_slice(&serde_json::to_vec(&shot).unwrap()).unwrap();
    assert_eq!(decoded, shot);
}
