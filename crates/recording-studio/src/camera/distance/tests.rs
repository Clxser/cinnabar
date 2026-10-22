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

#[test]
fn nested_curved_trims_preserve_every_sample_and_orientation_easing_without_resampling() {
    for timing in [PathTiming::Keyed, PathTiming::Distance] {
        let original = curved()
            .with_path_timing(timing)
            .unwrap()
            .with_key_easing(MediaTime(1_000_000), Easing::EaseIn)
            .unwrap();
        let first = original
            .trimmed(Span::new(MediaTime(200_000), MediaTime(900_000)).unwrap())
            .unwrap();
        let nested = first
            .trimmed(Span::new(MediaTime(300_000), MediaTime(800_000)).unwrap())
            .unwrap();
        let compiled = CompiledCameraPath::new(&nested);
        for value in [300_000, 310_000, 420_000, 500_000, 610_000, 799_999] {
            let at = MediaTime(value);
            assert_eq!(nested.sample(at, |_| None), original.sample(at, |_| None));
            assert_eq!(
                nested.sample_compiled(at, |_| None, compiled.as_ref()),
                original.sample(at, |_| None)
            );
        }
        assert!(nested.path_key_times().next().is_none());
        assert!(nested.key_at(MediaTime::ZERO).is_none());
        let points = nested.path_geometry_samples::<9>(0).unwrap();
        for (point, at) in [(points[0], nested.span.start), (points[8], nested.span.end)] {
            let expected = nested.path_pose(at).unwrap().position;
            assert!(DVec3::from_array(point).distance(DVec3::from_array(expected)) < 1e-10);
        }
        let encoded = serde_json::to_vec(&nested).unwrap();
        let decoded: CameraShot = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, nested);
    }
}

#[test]
fn moving_a_trimmed_shot_and_editing_visible_keys_use_the_original_path_clock() {
    let original = curved();
    let mut moved = original
        .trimmed(Span::new(MediaTime(300_000), MediaTime(800_000)).unwrap())
        .unwrap();
    moved.span = Span::new(MediaTime(3_000_000), MediaTime(3_500_000)).unwrap();
    moved.validate().unwrap();
    assert_eq!(
        moved.sample(MediaTime(3_100_000), |_| None),
        original.sample(MediaTime(400_000), |_| None)
    );
    let pose = CameraPose {
        position: [3.0, 4.0, 5.0],
        ..Default::default()
    };
    let edited = moved.keyed(MediaTime(3_100_000), pose).unwrap();
    assert_eq!(
        edited
            .path_key_times()
            .map(|(at, _)| at)
            .collect::<Vec<_>>(),
        vec![MediaTime(3_100_000)]
    );
    assert_eq!(
        edited.key_at(MediaTime(3_100_000)).unwrap().at,
        MediaTime(400_000)
    );
    assert_eq!(
        edited.parameter(CameraParameter::X, MediaTime(3_100_000)),
        Some(3.0)
    );
    let retimed = edited
        .with_key_time(MediaTime(3_100_000), MediaTime(3_200_000))
        .unwrap();
    assert_eq!(
        retimed
            .path_key_times()
            .map(|(at, _)| at)
            .collect::<Vec<_>>(),
        vec![MediaTime(3_200_000)]
    );
    assert_eq!(retimed.key_at(MediaTime(3_200_000)).unwrap().pose, pose);
}
