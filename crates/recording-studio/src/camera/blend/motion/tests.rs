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
