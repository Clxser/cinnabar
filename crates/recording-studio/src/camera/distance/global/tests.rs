use super::*;

/// Unequal spatial and temporal intervals distinguish whole-rail travel from per-key timing.
fn rail(points: &[[f64; 3]], times: &[u64]) -> CameraShot {
    assert_eq!(points.len(), times.len());
    let span = Span::new(MediaTime::ZERO, MediaTime(*times.last().unwrap())).unwrap();
    let mut shot = CameraShot::from_preview(
        Id(1),
        span,
        CameraRigKind::Path,
        CameraPose {
            position: points[0],
            ..Default::default()
        },
        None,
    )
    .unwrap();
    for (index, (&position, &at)) in points.iter().zip(times).enumerate().skip(1) {
        shot = shot
            .keyed(
                MediaTime(at),
                CameraPose {
                    position,
                    yaw: index as f64 * 40.0,
                    ..Default::default()
                },
            )
            .unwrap();
    }
    shot.with_path_smoothing(false)
        .unwrap()
        .with_path_timing(PathTiming::GlobalDistance)
        .unwrap()
}

/// Coordinate assertions compare behavior while allowing numerical integration roundoff.
fn near(actual: [f64; 3], expected: [f64; 3], tolerance: f64) {
    assert!(
        DVec3::from_array(actual).distance(DVec3::from_array(expected)) <= tolerance,
        "{actual:?} differs from {expected:?}"
    );
}

#[test]
fn unequal_key_lengths_do_not_change_whole_rail_speed() {
    let shot = rail(
        &[[0.0; 3], [10.0, 0.0, 0.0], [40.0, 0.0, 0.0]],
        &[0, 1_000_000, 2_000_000],
    );
    let cache = CompiledCameraPath::new(&shot).unwrap();
    for (at, x) in [
        (250_000, 5.0),
        (500_000, 10.0),
        (1_000_000, 20.0),
        (1_500_000, 30.0),
    ] {
        near(
            cache.sample(MediaTime(at)).unwrap().position,
            [x, 0.0, 0.0],
            1e-10,
        );
        near(
            cache.velocity(MediaTime(at)).unwrap(),
            [20.0, 0.0, 0.0],
            1e-10,
        );
    }
    let interval = shot.with_path_timing(PathTiming::Distance).unwrap();
    near(
        interval
            .sample(MediaTime(1_000_000), |_| None)
            .unwrap()
            .position,
        [10.0, 0.0, 0.0],
        1e-10,
    );
}

#[test]
fn global_easing_controls_acceleration_without_changing_lens_or_rotation_key_clocks() {
    let original = rail(
        &[[0.0; 3], [4.0, 0.0, 0.0], [20.0, 0.0, 0.0]],
        &[0, 1_000_000, 2_000_000],
    )
    .with_fov_key(MediaTime::ZERO, 70.0)
    .unwrap()
    .with_fov_key(MediaTime(2_000_000), 40.0)
    .unwrap()
    .with_fov_easing(MediaTime(2_000_000), Easing::EaseOut)
    .unwrap();
    let shot = original.with_path_global_easing(Easing::Smooth).unwrap();
    let cache = CompiledCameraPath::new(&shot).unwrap();
    for at in [0, 250_000, 500_000, 1_000_000, 1_500_000, 1_999_999] {
        let time = MediaTime(at);
        let amount = at as f64 / 2_000_000.0;
        let actual = shot.sample_compiled(time, |_| None, Some(&cache)).unwrap();
        let keyed = original
            .with_path_timing(PathTiming::Keyed)
            .unwrap()
            .sample(time, |_| None)
            .unwrap();
        near(
            actual.position,
            [20.0 * Easing::Smooth.sample(amount), 0.0, 0.0],
            1e-9,
        );
        assert_eq!(
            (actual.yaw, actual.pitch, actual.roll, actual.fov),
            (keyed.yaw, keyed.pitch, keyed.roll, keyed.fov)
        );
        near(
            cache.velocity(time).unwrap(),
            [10.0 * Easing::Smooth.derivative(amount), 0.0, 0.0],
            1e-9,
        );
    }
    near(cache.velocity(shot.span.start).unwrap(), [0.0; 3], 0.0);
    near(cache.velocity(shot.span.end).unwrap(), [0.0; 3], 0.0);
}

#[test]
fn aligned_join_derivatives_have_identical_direction_and_global_speed() {
    let shot = rail(
        &[[0.0; 3], [2.0, 0.0, 0.0], [2.0, 9.0, 0.0], [2.0, 9.0, 15.0]],
        &[0, 500_000, 1_500_000, 3_000_000],
    )
    .with_path_continuity(PathContinuity::Smooth)
    .unwrap();
    let cache = CompiledCameraPath::new(&shot).unwrap();
    let global = cache.global.as_ref().unwrap();
    assert!(global.stops.is_empty());
    let speed = cache.accuracy().distance / shot.span.duration().seconds();
    for index in 1..cache.segments.len() {
        let before = &cache.segments[index - 1];
        let after = &cache.segments[index];
        let incoming = (before.controls[3] - before.controls[2]) * 3.0;
        let outgoing = (after.controls[1] - after.controls[0]) * 3.0;
        assert!(incoming.length() > 0.0 && outgoing.length() > 0.0);
        // dP/dt = normalized(dP/du) * ds/dt on either side of an arc-length join.
        near(
            (incoming.normalize() * speed).to_array(),
            (outgoing.normalize() * speed).to_array(),
            1e-10,
        );
        let arrival =
            global.cumulative[index] / cache.accuracy().distance * shot.span.duration().0 as f64;
        let left = cache
            .velocity(MediaTime(arrival.round() as u64 - 1))
            .unwrap();
        let right = cache
            .velocity(MediaTime(arrival.round() as u64 + 1))
            .unwrap();
        near(left, right, 1e-3);
    }
}

#[test]
fn reversals_stop_instead_of_flipping_a_nonzero_velocity() {
    let shot = rail(
        &[[0.0; 3], [10.0, 0.0, 0.0], [0.0; 3]],
        &[0, 1_000_000, 2_000_000],
    )
    .with_path_continuity(PathContinuity::Smooth)
    .unwrap();
    let cache = CompiledCameraPath::new(&shot).unwrap();
    near(cache.velocity(MediaTime(1_000_000)).unwrap(), [0.0; 3], 0.0);
    let left = cache.velocity(MediaTime(999_900)).unwrap();
    let right = cache.velocity(MediaTime(1_000_100)).unwrap();
    assert!(left[0] >= 0.0 && right[0] <= 0.0);
    assert!(DVec3::from_array(left).length() < 0.01 && DVec3::from_array(right).length() < 0.01);
    let zone = &cache.global.as_ref().unwrap().stops[0];
    assert_eq!(zone.derivative(zone.center), 0.0);
    for edge in [zone.center - zone.half_width, zone.center + zone.half_width] {
        assert!((zone.sample(edge) - edge).abs() < 1e-14);
        assert!((zone.derivative(edge) - 1.0).abs() < 1e-12);
    }
}

#[test]
fn zero_length_sections_do_not_steal_distance_or_break_stationary_rails() {
    let shot = rail(
        &[[0.0; 3], [0.0; 3], [10.0, 0.0, 0.0], [10.0, 0.0, 0.0]],
        &[0, 1_000_000, 2_000_000, 3_000_000],
    );
    let cache = CompiledCameraPath::new(&shot).unwrap();
    near(
        cache.sample(MediaTime(750_000)).unwrap().position,
        [2.5, 0.0, 0.0],
        1e-10,
    );
    for at in [0, 750_000, 1_000_000, 2_500_000, 3_000_000] {
        cache.sample(MediaTime(at)).unwrap().validate().unwrap();
    }
    let still = rail(&[[2.0, 64.0, 3.0]; 3], &[0, 1_000_000, 2_000_000]);
    let cache = CompiledCameraPath::new(&still).unwrap();
    near(
        cache.sample(MediaTime(500_000)).unwrap().position,
        [2.0, 64.0, 3.0],
        0.0,
    );
    near(cache.velocity(MediaTime(500_000)).unwrap(), [0.0; 3], 0.0);
    assert!(cache.sample(MediaTime(500_000)).unwrap().yaw > 0.0);
}

#[test]
fn random_reverse_seeks_return_identical_samples_and_velocities() {
    let shot = rail(
        &[[0.0; 3], [3.0, 5.0, 0.0], [-2.0, 8.0, 10.0]],
        &[0, 400_000, 2_000_000],
    )
    .with_path_continuity(PathContinuity::Smooth)
    .unwrap()
    .with_path_global_easing(Easing::EaseIn)
    .unwrap();
    let cache = CompiledCameraPath::new(&shot).unwrap();
    let times = [
        0, 1, 250_000, 400_000, 700_000, 1_000_000, 1_999_999, 2_000_000,
    ];
    let expected = times.map(|at| (cache.sample(MediaTime(at)), cache.velocity(MediaTime(at))));
    for index in [7, 2, 5, 0, 6, 1, 3, 4, 7, 0] {
        assert_eq!(
            (
                cache.sample(MediaTime(times[index])),
                cache.velocity(MediaTime(times[index]))
            ),
            expected[index]
        );
    }
}

#[test]
fn global_acceleration_and_geometry_survive_nested_trims_moves_and_serialization() {
    let shot = rail(
        &[[0.0; 3], [3.0, 5.0, 0.0], [-2.0, 8.0, 10.0]],
        &[0, 400_000, 2_000_000],
    )
    .with_path_continuity(PathContinuity::Smooth)
    .unwrap()
    .with_path_global_easing(Easing::EaseOut)
    .unwrap();
    let original = CompiledCameraPath::new(&shot).unwrap();
    let trimmed = shot
        .trimmed(Span::new(MediaTime(150_000), MediaTime(1_800_000)).unwrap())
        .unwrap()
        .trimmed(Span::new(MediaTime(300_000), MediaTime(1_400_000)).unwrap())
        .unwrap();
    let mut moved = trimmed.clone();
    moved.span = Span::new(MediaTime(4_000_000), MediaTime(5_100_000)).unwrap();
    let moved: CameraShot = serde_json::from_slice(&serde_json::to_vec(&moved).unwrap()).unwrap();
    moved.validate().unwrap();
    let cache = CompiledCameraPath::new(&moved).unwrap();
    for delta in [0, 100_000, 300_000, 700_000, 1_099_999] {
        assert_eq!(
            cache.sample(MediaTime(4_000_000 + delta)),
            original.sample(MediaTime(300_000 + delta))
        );
        assert_eq!(
            cache.velocity(MediaTime(4_000_000 + delta)),
            original.velocity(MediaTime(300_000 + delta))
        );
    }
    assert_eq!(
        moved.path_key_times().map(|(at, _)| at).collect::<Vec<_>>(),
        vec![MediaTime(4_100_000)]
    );
    let samples = (0..moved.path_keys().len() - 1)
        .find_map(|index| cache.geometry_samples::<9>(index))
        .unwrap();
    near(
        samples[0],
        cache.sample(moved.span.start).unwrap().position,
        1e-9,
    );
}

#[test]
fn length_estimates_are_explicit_and_stay_between_chord_and_control_polygon_bounds() {
    let shot = rail(
        &[[0.0; 3], [4.0, 6.0, 0.0], [10.0, 2.0, 8.0]],
        &[0, 500_000, 2_000_000],
    )
    .with_path_continuity(PathContinuity::Smooth)
    .unwrap();
    let cache = CompiledCameraPath::new(&shot).unwrap();
    let lower = cache
        .segments
        .iter()
        .map(|segment| segment.controls[0].distance(segment.controls[3]))
        .sum::<f64>();
    let upper = cache
        .segments
        .iter()
        .map(|segment| {
            segment
                .controls
                .windows(2)
                .map(|pair| pair[0].distance(pair[1]))
                .sum::<f64>()
        })
        .sum::<f64>();
    assert!(
        cache.accuracy().distance >= lower - 1e-10 && cache.accuracy().distance <= upper + 1e-10
    );
    assert!(cache.accuracy().estimated_length_error.is_finite());
    assert!(cache.accuracy().estimated_length_error < cache.accuracy().distance * 1e-5);
}

#[test]
fn legacy_documents_keep_original_geometry_and_keyed_easing_exactly() {
    let original = rail(
        &[[0.0; 3], [4.0, 6.0, 0.0], [10.0, 2.0, 8.0]],
        &[0, 500_000, 2_000_000],
    )
    .with_path_timing(PathTiming::Keyed)
    .unwrap();
    let mut value = serde_json::to_value(&original).unwrap();
    for field in ["path_continuity", "path_global_easing"] {
        value.as_object_mut().unwrap().remove(field);
    }
    let legacy: CameraShot = serde_json::from_value(value).unwrap();
    assert_eq!(legacy.path_continuity, PathContinuity::Authored);
    assert_eq!(legacy.path_global_easing, Easing::Linear);
    for at in [0, 250_000, 499_999, 500_000, 1_500_000, 1_999_999] {
        assert_eq!(
            legacy.sample(MediaTime(at), |_| None),
            original.sample(MediaTime(at), |_| None)
        );
    }
}
