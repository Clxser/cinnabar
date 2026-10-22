use super::*;

#[test]
fn immediate_defaults_match_fly_exactly_and_speed_is_reversible() {
    let pose = CameraPose {
        yaw: 42.0,
        pitch: 25.0,
        roll: 11.0,
        ..Default::default()
    };
    let settings = NavigationSettings::default();
    assert_eq!(
        settings.adjusted(4.0).unwrap().adjusted(-4.0).unwrap(),
        settings
    );
    for fast in [false, true] {
        let mut motion = NavigationMotion::default();
        assert_eq!(
            motion.step(pose, [1.0; 3], [2.0, -3.0], 0.03, settings, fast),
            pose.flown(
                [1.0; 3],
                [2.0, -3.0],
                0.03,
                DEFAULT_SPEED * if fast { FAST_MULTIPLIER } else { 1.0 }
            )
        );
    }
}

#[test]
fn analytic_translation_is_partition_invariant_and_release_has_bounded_tail() {
    let settings = NavigationSettings {
        smooth: true,
        ..Default::default()
    };
    let pose = CameraPose::default();
    let mut whole = NavigationMotion::default();
    let expected = whole
        .step(pose, [0.0, 0.0, 1.0], [0.0; 2], 0.1, settings, false)
        .unwrap();
    let mut split = NavigationMotion::default();
    let mut actual = pose;
    for _ in 0..10 {
        actual = split
            .step(actual, [0.0, 0.0, 1.0], [0.0; 2], 0.01, settings, false)
            .unwrap();
    }
    assert!((actual.position[2] - expected.position[2]).abs() < 1e-12);
    assert!((split.velocity - whole.velocity).length() < 1e-12);
    assert!(actual.position[2] > 0.0 && actual.position[2] < DEFAULT_SPEED * 0.1);
    let mut last = actual.position[2];
    for _ in 0..200 {
        actual = split
            .step(actual, [0.0; 3], [0.0; 2], 0.1, settings, false)
            .unwrap();
        assert!(actual.position[2] >= last);
        last = actual.position[2];
    }
    assert_eq!(split.velocity, DVec3::ZERO);
    let stopped = split
        .step(actual, [0.0; 3], [0.0; 2], 0.1, settings, false)
        .unwrap();
    assert_eq!(stopped, actual);
    split.reset();
    assert_eq!(split, NavigationMotion::default());
}

#[test]
fn dolly_zoom_pan_and_level_preserve_unrelated_pose_fields() {
    let pose = CameraPose {
        yaw: 35.0,
        pitch: -15.0,
        roll: 27.0,
        fov: 90.0,
        ..Default::default()
    };
    let pivot = pose.orbit_anchor(8.0).unwrap();
    let panned = pose.panned(pivot, 1.7, [0.15, -0.2]).unwrap();
    let point = CameraProjection::new(panned, 1.7)
        .unwrap()
        .project(pivot)
        .unwrap();
    assert!((point[0] - 0.65).abs() < 1e-12 && (point[1] - 0.3).abs() < 1e-12);
    for moved in [panned, pose.dollied(3.0).unwrap()] {
        assert_eq!(
            (moved.yaw, moved.pitch, moved.roll, moved.fov),
            (pose.yaw, pose.pitch, pose.roll, pose.fov)
        );
    }
    let restored = pose.dollied(3.0).unwrap().dollied(-3.0).unwrap();
    assert!(
        DVec3::from_array(restored.position).distance(DVec3::from_array(pose.position)) < 1e-12
    );
    let zoom = pose.zoomed(4.0).unwrap();
    assert_eq!(zoom.position, pose.position);
    assert!((zoom.zoomed(-4.0).unwrap().fov - pose.fov).abs() < 1e-12);
    assert!(zoom.fov < pose.fov);
    assert_eq!(pose.leveled(), CameraPose { roll: 0.0, ..pose });
}

#[test]
fn invalid_navigation_never_poison_state_and_zero_time_does_not_translate() {
    let mut motion = NavigationMotion::default();
    let settings = NavigationSettings {
        smooth: true,
        ..Default::default()
    };
    let pose = CameraPose::default();
    for elapsed in [f64::NAN, f64::INFINITY, -1.0] {
        assert!(
            motion
                .step(pose, [1.0; 3], [0.0; 2], elapsed, settings, false)
                .is_none()
        );
        assert_eq!(motion, NavigationMotion::default());
    }
    assert_eq!(
        motion.step(pose, [1.0; 3], [0.0; 2], 0.0, settings, false),
        Some(pose)
    );
    assert!(pose.zoomed(f64::NAN).is_none());
    assert!(pose.dollied(f64::INFINITY).is_none());
    assert!(pose.panned([f64::NAN; 3], 1.0, [0.0; 2]).is_none());
}
