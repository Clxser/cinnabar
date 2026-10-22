use super::super::tests::{project, sample};
use super::*;
use crate::{
    effect::{Curve, CurveKey},
    playback::{CompiledTimeline, FramePlan},
};

/// Backward and forward rotation increments use world-space shortest quaternion logarithms.
fn angular_delta(from: CameraPose, to: CameraPose, seconds: f64) -> DVec3 {
    let mut delta = orientation(to) * orientation(from).conjugate();
    if delta.w < 0.0 {
        delta = -delta;
    }
    delta.to_scaled_axis() / seconds
}

/// A ten-microsecond stencil checks both pose and velocity without depending on a playback history.
fn assert_seam(sample: impl Fn(u64) -> CameraPose, at: u64) {
    let step = 10;
    let seconds = MediaTime(step).seconds();
    let before = sample(at - step);
    let center = sample(at);
    let after = sample(at + step);
    let left = (DVec3::from_array(center.position) - DVec3::from_array(before.position)) / seconds;
    let right = (DVec3::from_array(after.position) - DVec3::from_array(center.position)) / seconds;
    assert!(
        (left - right).length() < 0.01,
        "position velocity {left:?} -> {right:?}"
    );
    let left = angular_delta(before, center, seconds);
    let right = angular_delta(center, after, seconds);
    assert!(
        (left - right).length() < 0.002,
        "angular velocity {left:?} -> {right:?}"
    );
    let left = (center.fov - before.fov) / seconds;
    let right = (after.fov - center.fov) / seconds;
    assert!(
        (left - right).abs() < 0.02,
        "lens velocity {left} -> {right}"
    );
}

#[test]
fn smooth_entries_retain_terminal_linear_angular_and_lens_velocity_and_incoming_end_velocity() {
    for timing in [
        PathTiming::Keyed,
        PathTiming::Distance,
        PathTiming::GlobalDistance,
    ] {
        let mut project = project();
        for (index, start, finish) in [
            (
                0,
                CameraPose {
                    position: [0.0, 64.0, 0.0],
                    yaw: 350.0,
                    pitch: 10.0,
                    roll: -20.0,
                    fov: 70.0,
                },
                CameraPose {
                    position: [4.0, 66.0, 2.0],
                    yaw: 10.0,
                    pitch: 20.0,
                    roll: 20.0,
                    fov: 90.0,
                },
            ),
            (
                1,
                CameraPose {
                    position: [10.0, 70.0, -3.0],
                    yaw: 25.0,
                    pitch: -10.0,
                    roll: 5.0,
                    fov: 60.0,
                },
                CameraPose {
                    position: [13.0, 70.0, 0.0],
                    yaw: 55.0,
                    pitch: -5.0,
                    roll: -5.0,
                    fov: 75.0,
                },
            ),
        ] {
            let original = project.shots[index].clone();
            project.shots[index] = CameraShot::from_preview(
                original.id,
                original.span,
                CameraRigKind::Path,
                start,
                None,
            )
            .unwrap()
            .keyed(original.span.end, finish)
            .unwrap()
            .with_key_easing(original.span.end, Easing::Linear)
            .unwrap()
            .with_path_smoothing(false)
            .unwrap()
            .with_path_timing(timing)
            .unwrap();
            project.shots[index].blend_in = original.blend_in;
        }
        project.shots[0].fov_curve = Some(Curve {
            keys: vec![
                CurveKey {
                    at: MediaTime::ZERO,
                    value: 70.0,
                    easing: Easing::Linear,
                },
                CurveKey {
                    at: MediaTime(2_000_000),
                    value: 90.0,
                    easing: Easing::EaseIn,
                },
            ],
        });
        let timeline = CompiledTimeline::new(Arc::new(project.clone())).unwrap();
        let at = |time| {
            let mut frame = FramePlan::default();
            timeline
                .sample(MediaTime(time), &mut frame, |_, _, _| None)
                .unwrap();
            frame.base_camera.unwrap()
        };
        assert_seam(at, 2_000_000);
        assert_seam(at, 3_000_000);
        assert_eq!(
            at(3_000_000),
            project.shots[1]
                .sample(MediaTime(3_000_000), |_| None)
                .unwrap()
        );
        // Reverse/random access uses no prior samples to carry terminal velocity.
        let center = at(2_500_000);
        let _ = at(3_000_000);
        let _ = at(2_000_000);
        assert_eq!(center, at(2_500_000));
    }
}

#[test]
fn an_orbit_continues_authored_velocity_while_the_recorded_subject_keeps_moving() {
    let mut project = project();
    let target = super::super::tests::moving_target();
    for (index, kind, position) in [
        (0, CameraRigKind::Orbit, [0.0, 66.0, -5.0]),
        (1, CameraRigKind::Follow, [8.0, 68.0, -2.0]),
    ] {
        let original = project.shots[index].clone();
        project.shots[index] = CameraShot::from_preview(
            original.id,
            original.span,
            kind,
            CameraPose {
                position,
                ..Default::default()
            },
            Some(&target),
        )
        .unwrap();
        project.shots[index].blend_in = original.blend_in;
    }
    let timeline = CompiledTimeline::new(Arc::new(project)).unwrap();
    assert_seam(
        |time| {
            let mut frame = FramePlan::default();
            timeline
                .sample(MediaTime(time), &mut frame, |_, reference, at| {
                    assert_eq!(reference, target.reference());
                    Some(ActorPose {
                        position: [at.seconds() * 2.0, 64.0, 0.0],
                        yaw: at.seconds() * 4.0,
                        ..target.pose
                    })
                })
                .unwrap();
            frame.base_camera.unwrap()
        },
        2_000_000,
    );
}

#[test]
fn intentional_hold_jumps_do_not_become_unbounded_extrapolated_motion() {
    let mut project = project();
    let original = project.shots[0].clone();
    project.shots[0] = CameraShot::from_preview(
        original.id,
        original.span,
        CameraRigKind::Path,
        CameraPose::default(),
        None,
    )
    .unwrap()
    .keyed(
        original.span.end,
        CameraPose {
            position: [4.0, 64.0, 0.0],
            yaw: 120.0,
            roll: 30.0,
            fov: 100.0,
            ..Default::default()
        },
    )
    .unwrap()
    .with_key_easing(original.span.end, Easing::Hold)
    .unwrap();
    let outgoing = &project.shots[0];
    let terminal = outgoing.sample_terminal(|_| None, None).unwrap();
    let motion = TerminalMotion::new(outgoing, None);
    assert_eq!(motion.extrapolated(terminal, MediaTime(500_000)), terminal);
    let expected = project.shots[1].blend_in.unwrap().sample(
        terminal,
        project.shots[1]
            .sample(MediaTime(2_500_000), |_| None)
            .unwrap(),
        MediaTime(500_000),
    );
    assert_eq!(sample(&project, 2_500_000).base_camera, Some(expected));
}

/// Serde defaults keep older absolute curves compatible when the additive schema grows.
fn add_channel(
    shot: &mut CameraShot,
    channel: CameraChannel,
    curve: Curve,
    offset: MediaTime,
    additive: bool,
) {
    let mut value = serde_json::json!({"curve":curve,"offset":offset});
    if additive {
        value["additive"] = serde_json::Value::Bool(true);
    }
    shot.animation
        .curves
        .insert(channel, serde_json::from_value(value).unwrap());
}

/// One linear scalar interval supplies an exact authored rate independent of its owning rig.
fn linear_curve(start: f64, end: f64, duration: MediaTime) -> Curve {
    Curve {
        keys: vec![
            CurveKey {
                at: MediaTime::ZERO,
                value: start,
                easing: Easing::Linear,
            },
            CurveKey {
                at: duration,
                value: end,
                easing: Easing::Linear,
            },
        ],
    }
}

#[test]
fn outgoing_rails_combine_graph_rates_with_base_distance_velocity_per_axis() {
    for timing in [
        PathTiming::Keyed,
        PathTiming::Distance,
        PathTiming::GlobalDistance,
    ] {
        for additive in [false, true] {
            let mut project = project();
            let original = project.shots[0].clone();
            let outgoing = &mut project.shots[0];
            *outgoing = CameraShot::from_preview(
                original.id,
                original.span,
                CameraRigKind::Path,
                CameraPose::default(),
                None,
            )
            .unwrap()
            .keyed(
                original.span.end,
                CameraPose {
                    position: [4.0, 66.0, 2.0],
                    ..Default::default()
                },
            )
            .unwrap()
            .with_key_easing(original.span.end, Easing::Linear)
            .unwrap()
            .with_path_smoothing(false)
            .unwrap()
            .with_path_timing(timing)
            .unwrap();
            let duration = outgoing.span.duration();
            add_channel(
                outgoing,
                CameraChannel::X,
                linear_curve(0.0, 6.0, duration),
                MediaTime::ZERO,
                additive,
            );
            add_channel(
                outgoing,
                CameraChannel::Yaw,
                linear_curve(350.0, 390.0, duration),
                MediaTime::ZERO,
                additive,
            );
            add_channel(
                outgoing,
                CameraChannel::Pitch,
                linear_curve(10.0, 20.0, duration),
                MediaTime::ZERO,
                additive,
            );
            add_channel(
                outgoing,
                CameraChannel::Roll,
                linear_curve(0.0, 20.0, duration),
                MediaTime::ZERO,
                additive,
            );
            let path = CompiledCameraPath::new(outgoing);
            let motion = TerminalMotion::new(outgoing, path.as_ref());
            assert!((motion.linear.x - if additive { 5.0 } else { 3.0 }).abs() < 1e-8);
            assert!((motion.linear.y - 1.0).abs() < 1e-8);
            assert!((motion.linear.z - 1.0).abs() < 1e-8);
            if let Some(path) = path {
                let exact = path.velocity(MediaTime(outgoing.span.end.0 - 1)).unwrap();
                assert_eq!([motion.linear.y, motion.linear.z], [exact[1], exact[2]]);
            }
            let timeline = CompiledTimeline::new(Arc::new(project)).unwrap();
            assert_seam(
                |at| {
                    let mut frame = FramePlan::default();
                    timeline
                        .sample(MediaTime(at), &mut frame, |_, _, _| None)
                        .unwrap();
                    frame.base_camera.unwrap()
                },
                2_000_000,
            );
        }
    }
}

#[test]
fn outgoing_orbit_graph_channels_replace_or_add_to_analytic_authored_motion() {
    let target = super::super::tests::moving_target();
    for additive in [false, true] {
        let mut project = project();
        let original = project.shots[0].clone();
        let outgoing = &mut project.shots[0];
        *outgoing = CameraShot::from_preview(
            original.id,
            original.span,
            CameraRigKind::Orbit,
            CameraPose {
                position: [0.0, 66.0, -5.0],
                ..Default::default()
            },
            Some(&target),
        )
        .unwrap();
        let base = TerminalMotion::new(outgoing, None);
        let duration = outgoing.span.duration();
        add_channel(
            outgoing,
            CameraChannel::Z,
            linear_curve(0.0, 6.0, duration),
            MediaTime::ZERO,
            additive,
        );
        add_channel(
            outgoing,
            CameraChannel::Yaw,
            linear_curve(0.0, 40.0, duration),
            MediaTime::ZERO,
            additive,
        );
        add_channel(
            outgoing,
            CameraChannel::Roll,
            linear_curve(0.0, 20.0, duration),
            MediaTime::ZERO,
            additive,
        );
        let motion = TerminalMotion::new(outgoing, None);
        assert_eq!(motion.linear.x, base.linear.x);
        assert!(
            (motion.linear.z - (if additive { base.linear.z } else { 0.0 }) - 3.0).abs() < 1e-8
        );
        assert!(
            (motion.angular.x - (if additive { base.angular.x } else { 0.0 }) - 20.0).abs() < 1e-8
        );
        let timeline = CompiledTimeline::new(Arc::new(project)).unwrap();
        assert_seam(
            |at| {
                let mut frame = FramePlan::default();
                timeline
                    .sample(MediaTime(at), &mut frame, |_, reference, time| {
                        assert_eq!(reference, target.reference());
                        Some(ActorPose {
                            position: [time.seconds() * 1.75, 64.0, 0.0],
                            ..target.pose
                        })
                    })
                    .unwrap();
                frame.base_camera.unwrap()
            },
            2_000_000,
        );
    }
}

#[test]
fn target_bound_graph_rotation_uses_actual_terminal_axes_and_keeps_current_subject_motion() {
    let target = super::super::tests::moving_target();
    for kind in [
        CameraRigKind::Player,
        CameraRigKind::Track,
        CameraRigKind::Follow,
    ] {
        for additive in [false, true] {
            let mut project = project();
            let original = project.shots[0].clone();
            let outgoing = &mut project.shots[0];
            *outgoing = CameraShot::from_preview(
                original.id,
                original.span,
                kind,
                CameraPose {
                    position: [8.0, 68.0, -5.0],
                    roll: 30.0,
                    ..Default::default()
                },
                Some(&target),
            )
            .unwrap();
            let duration = outgoing.span.duration();
            for (channel, rate) in [
                (CameraChannel::X, 2.0),
                (CameraChannel::Yaw, 3.0),
                (CameraChannel::Pitch, 5.0),
                (CameraChannel::Roll, 7.0),
            ] {
                add_channel(
                    outgoing,
                    channel,
                    linear_curve(0.0, rate * duration.seconds(), duration),
                    MediaTime::ZERO,
                    additive,
                );
            }
            let timeline = CompiledTimeline::new(Arc::new(project)).unwrap();
            assert_seam(
                |at| {
                    let mut frame = FramePlan::default();
                    timeline
                        .sample(MediaTime(at), &mut frame, |_, reference, time| {
                            assert_eq!(reference, target.reference());
                            Some(ActorPose {
                                position: [time.seconds() * 1.75, 64.0, 0.0],
                                yaw: 100.0 + time.seconds() * 4.0,
                                pitch: 40.0,
                                roll: 30.0,
                                ..target.pose
                            })
                        })
                        .unwrap();
                    frame.base_camera.unwrap()
                },
                2_000_000,
            );
        }
    }
}

#[test]
fn graph_phase_limits_stencil_and_hold_rates_preserve_only_additive_base_motion() {
    let mut project = project();
    let outgoing = &mut project.shots[0];
    let original = outgoing.clone();
    *outgoing = CameraShot::from_preview(
        original.id,
        original.span,
        CameraRigKind::Path,
        CameraPose::default(),
        None,
    )
    .unwrap()
    .keyed(
        original.span.end,
        CameraPose {
            position: [4.0, 64.0, 0.0],
            ..Default::default()
        },
    )
    .unwrap()
    .with_key_easing(original.span.end, Easing::Linear)
    .unwrap()
    .with_path_smoothing(false)
    .unwrap();
    let curve = Curve {
        keys: vec![
            CurveKey {
                at: MediaTime(2_999_990),
                value: 0.0,
                easing: Easing::Linear,
            },
            CurveKey {
                at: MediaTime(3_000_000),
                value: 4.0,
                easing: Easing::Hold,
            },
        ],
    };
    for additive in [false, true] {
        add_channel(
            outgoing,
            CameraChannel::X,
            curve.clone(),
            MediaTime(1_000_000),
            additive,
        );
        assert_eq!(derivative_step(outgoing), MediaTime(5));
        let motion = TerminalMotion::new(outgoing, None);
        assert!((motion.linear.x - if additive { 2.0 } else { 0.0 }).abs() < 1e-7);
    }
}

#[test]
fn derivative_preparation_cannot_fabricate_a_player_eye_or_predict_a_subject() {
    let mut project = project();
    let target = super::super::tests::moving_target();
    let original = project.shots[0].clone();
    let outgoing = &mut project.shots[0];
    *outgoing = CameraShot::from_preview(
        original.id,
        original.span,
        CameraRigKind::Player,
        CameraPose::default(),
        Some(&target),
    )
    .unwrap();
    let duration = outgoing.span.duration();
    add_channel(
        outgoing,
        CameraChannel::X,
        linear_curve(0.0, 4.0, duration),
        MediaTime::ZERO,
        true,
    );
    let motion = TerminalMotion::new(outgoing, None);
    assert_eq!(motion.linear.x, 2.0);
    let expected = project.shots[1].sample(MediaTime(2_500_000), |_| None);
    let timeline = CompiledTimeline::new(Arc::new(project)).unwrap();
    let mut frame = FramePlan::default();
    timeline
        .sample(MediaTime(2_500_000), &mut frame, |_, reference, time| {
            assert_eq!(reference, target.reference());
            assert_eq!(time, MediaTime(2_500_000));
            Some(ActorPose {
                eye_offset: None,
                ..target.pose
            })
        })
        .unwrap();
    assert_eq!(frame.base_camera, expected);
}

#[test]
fn weighted_graph_rates_match_the_saved_curve_at_trimmed_source_phases() {
    let mut curve = linear_curve(1.0, 7.0, MediaTime(2_000_000));
    curve.keys[1].easing = Easing::Bezier {
        outgoing: [15_000, 9_000],
        incoming: [43_000, 50_000],
    };
    for at in [500_000, 1_000_000, 1_500_000, 2_000_000] {
        let rate = (curve.sample(MediaTime(at)) - curve.sample(MediaTime(at - 1)))
            / MediaTime(1).seconds();
        assert!((curve_rate(&curve, MediaTime(at)) - rate).abs() < 0.001);
    }
    assert_eq!(curve_rate(&curve, MediaTime::ZERO), 0.0);
    assert_eq!(curve_rate(&curve, MediaTime(2_000_001)), 0.0);
}
