use super::*;
use crate::{
    PlaybackRate,
    effect::{Curve, EffectClip, EffectKind},
    model::{Clip, Framing, MediaSource},
    playback::{CompiledTimeline, FramePlan},
};
use std::cell::RefCell;

/// All media is anonymous metadata: blend sampling needs no recording, filesystem, worker or clock.
pub(super) fn project() -> Project {
    let mut project = Project::new("Camera entries");
    project.sources.push(MediaSource {
        id: Id(1),
        name: "Recorded scene".into(),
        path: "scene.cinreplay".into(),
        kind: MediaKind::Replay,
        duration: MediaTime(8_000_000),
        size: [1280, 720],
        frame_rate: None,
        sha256: None,
    });
    project.clips.push(Clip {
        id: Id(2),
        source: Id(1),
        source_range: Span::new(MediaTime::ZERO, MediaTime(8_000_000)).unwrap(),
        at: MediaTime::ZERO,
        rate: PlaybackRate::default(),
        speed_ramp: None,
        track: 0,
        framing: Framing::default(),
        enabled: true,
    });
    project.shots = [
        shot(
            3,
            0,
            2_000_000,
            CameraPose {
                position: [0.0, 64.0, 0.0],
                yaw: 359.0,
                fov: 90.0,
                ..Default::default()
            },
        ),
        shot(
            4,
            2_000_000,
            5_000_000,
            CameraPose {
                position: [10.0, 64.0, 0.0],
                yaw: 1.0,
                fov: 60.0,
                ..Default::default()
            },
        ),
    ]
    .into();
    project.shots[1].blend_in = Some(CameraBlend {
        duration: MediaTime(1_000_000),
        easing: Easing::Smooth,
    });
    project
}

/// Ordinary authoring constructors own every unrelated default field.
fn shot(id: u64, start: u64, end: u64, pose: CameraPose) -> CameraShot {
    CameraShot::from_preview(
        Id(id),
        Span::new(MediaTime(start), MediaTime(end)).unwrap(),
        CameraRigKind::Free,
        pose,
        None,
    )
    .unwrap()
}

/// Plans are independent of seek direction and use exactly the callback supplied by preview/export.
pub(super) fn sample(project: &Project, at: u64) -> FramePlan {
    let mut frame = FramePlan::default();
    CompiledTimeline::new(Arc::new(project.clone()))
        .unwrap()
        .sample(MediaTime(at), &mut frame, |_, _, _| None)
        .unwrap();
    frame
}

/// Equivalent quaternion rotations may have different Euler representatives near a pole.
fn assert_orientation(actual: CameraPose, expected: CameraPose) {
    assert!(orientation(actual).dot(orientation(expected)).abs() > 1.0 - 1e-10);
}

#[test]
fn exact_cut_legacy_defaults_saved_blends_and_admission() {
    let mut project = project();
    assert_eq!(
        Project::from_json(&project.to_json().unwrap()).unwrap(),
        project
    );
    let incoming = project.shots[1].clone();
    let value = serde_json::to_value(&incoming).unwrap();
    assert_eq!(
        serde_json::from_value::<CameraShot>(value).unwrap(),
        incoming
    );
    project.shots[1].blend_in = None;
    let legacy = serde_json::to_value(&project.shots[1]).unwrap();
    assert!(legacy.get("blend_in").is_none());
    assert!(
        serde_json::from_value::<CameraShot>(legacy)
            .unwrap()
            .blend_in
            .is_none()
    );
    assert_eq!(
        sample(&project, 2_000_000).camera,
        incoming.sample(MediaTime(2_000_000), |_| None)
    );
    for duration in [MediaTime::ZERO, MediaTime(3_000_001)] {
        assert!(
            incoming
                .with_blend_in(Some(CameraBlend {
                    duration,
                    easing: Easing::Smooth
                }))
                .is_err()
        );
    }
}

#[test]
fn adjacent_free_rigs_have_exact_endpoints_short_arc_lens_and_random_access() {
    let project = project();
    let terminal = project.shots[0].sample_terminal(|_| None, None).unwrap();
    assert_eq!(sample(&project, 2_000_000).camera, Some(terminal));
    let middle = sample(&project, 2_500_000).camera.unwrap();
    assert_eq!(middle.position, [5.0, 64.0, 0.0]);
    assert_eq!(middle.fov, 75.0);
    assert_orientation(middle, CameraPose { yaw: 0.0, ..middle });
    assert_eq!(
        sample(&project, 3_000_000).camera,
        project.shots[1].sample(MediaTime(3_000_000), |_| None)
    );
    let timeline = CompiledTimeline::new(Arc::new(project)).unwrap();
    let mut frame = FramePlan::default();
    let mut reference = BTreeMap::new();
    for time in [
        2_000_000, 2_100_000, 2_250_000, 2_500_000, 2_999_999, 3_000_000,
    ] {
        timeline
            .sample(MediaTime(time), &mut frame, |_, _, _| None)
            .unwrap();
        reference.insert(time, frame.camera);
    }
    for time in [
        3_000_000, 2_999_999, 2_100_000, 2_500_000, 2_000_000, 2_250_000,
    ] {
        timeline
            .sample(MediaTime(time), &mut frame, |_, _, _| None)
            .unwrap();
        assert_eq!(frame.camera, reference[&time]);
    }
}

#[test]
fn quaternion_blends_cross_wrap_and_near_poles_without_long_rotations() {
    let blend = CameraBlend {
        duration: MediaTime(1_000),
        easing: Easing::Linear,
    };
    for (from, to) in [
        (
            CameraPose {
                yaw: 179.0,
                roll: 179.0,
                ..Default::default()
            },
            CameraPose {
                yaw: -179.0,
                roll: -179.0,
                ..Default::default()
            },
        ),
        (
            CameraPose {
                yaw: 350.0,
                pitch: 89.999,
                roll: 10.0,
                ..Default::default()
            },
            CameraPose {
                yaw: 10.0,
                pitch: 90.0,
                roll: -10.0,
                ..Default::default()
            },
        ),
        (
            CameraPose {
                yaw: 15.0,
                pitch: -90.0,
                roll: 25.0,
                ..Default::default()
            },
            CameraPose {
                yaw: 35.0,
                pitch: -89.999,
                roll: 45.0,
                ..Default::default()
            },
        ),
    ] {
        assert_eq!(blend.sample(from, to, MediaTime::ZERO), from);
        assert_eq!(blend.sample(from, to, blend.duration), to);
        let mut previous = orientation(from);
        for at in 1..=1000 {
            let pose = blend.sample(from, to, MediaTime(at));
            pose.validate().unwrap();
            let current = orientation(pose);
            assert!(previous.dot(current).abs() > 0.999);
            previous = current;
        }
        assert!(
            orientation(blend.sample(from, to, MediaTime(500)))
                .dot(orientation(from).slerp(orientation(to), 0.5))
                .abs()
                > 1.0 - 1e-10
        );
    }
}

#[test]
fn gaps_overlaps_ambiguous_layers_source_cuts_and_nonreplays_stay_exact_cuts() {
    for incompatible in 0..7 {
        let mut project = project();
        match incompatible {
            0 => project.shots[0].span.end = MediaTime(1_900_000),
            1 => project.shots[0].span.end = MediaTime(2_100_000),
            2 => project.sources[0].kind = MediaKind::Video,
            3 => {
                let mut other = project.clips[0].clone();
                other.id = Id(5);
                other.track = 1;
                project.clips.push(other);
            }
            4 => {
                let mut next = project.clips[0].clone();
                project.clips[0].source_range.end = MediaTime(2_000_000);
                next.id = Id(5);
                next.at = MediaTime(2_000_000);
                next.source_range.start = MediaTime(2_000_000);
                project.clips.push(next);
            }
            5 => {
                let mut next = project.clips[0].clone();
                project.clips[0].source_range.end = MediaTime(2_000_000);
                next.id = Id(6);
                next.at = MediaTime(2_000_000);
                next.source_range.start = MediaTime(2_000_000);
                next.source = Id(5);
                let mut source = project.sources[0].clone();
                source.id = Id(5);
                source.path = "different.cinreplay".into();
                project.sources.push(source);
                project.clips.push(next);
            }
            _ => project
                .shots
                .push(shot(5, 2_200_000, 2_300_000, CameraPose::default())),
        }
        let incoming = project.shots[1].sample(MediaTime(2_500_000), |_| None);
        assert_eq!(
            sample(&project, 2_500_000).camera,
            incoming,
            "case {incompatible}"
        );
    }
}

#[test]
fn terminal_path_pose_includes_its_last_key_and_lens_before_global_effects() {
    let mut project = project();
    let span = project.shots[0].span;
    project.shots[0] = CameraShot::from_preview(
        Id(3),
        span,
        CameraRigKind::Path,
        CameraPose::default(),
        None,
    )
    .unwrap()
    .keyed(
        span.end,
        CameraPose {
            position: [4.0, 65.0, 2.0],
            yaw: 45.0,
            ..Default::default()
        },
    )
    .unwrap()
    .with_fov_key(span.end, 54.0)
    .unwrap()
    .with_path_timing(PathTiming::GlobalDistance)
    .unwrap();
    project.effects.push(EffectClip {
        id: Id(5),
        span: Span::new(MediaTime::ZERO, MediaTime(5_000_000)).unwrap(),
        kind: EffectKind::Zoom,
        amount: Curve::constant(5.0),
        enabled: true,
        origin: None,
    });
    let frame = sample(&project, 2_000_000);
    let endpoint = project.shots[0].sample_terminal(|_| None, None).unwrap();
    assert_eq!(endpoint.position, [4.0, 65.0, 2.0]);
    assert_eq!(endpoint.fov, 54.0);
    assert_eq!(frame.base_camera, Some(endpoint));
    assert_eq!(
        frame.camera,
        Some(frame.effects.camera_pose(endpoint, MediaTime(2_000_000)))
    );
}

/// A recorded subject has an explicit spawn lifetime; all lookup poses come from the caller's current time.
pub(super) fn moving_target() -> CameraTarget {
    CameraTarget {
        view: Default::default(),
        runtime_id: 7,
        unique_id: 70,
        spawn_revision: 2,
        name: "Subject".into(),
        kind: CameraTargetKind::Player,
        entity_type: None,
        teleported: false,
        pose: ActorPose {
            position: [0.0, 64.0, 0.0],
            velocity: [1.0, 0.0, 0.0],
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            eye_offset: Some([0.0, 1.6, 0.0]),
        },
    }
}

#[test]
fn outgoing_follow_keeps_moving_at_current_verified_retimed_source_time() {
    let mut project = project();
    let target = moving_target();
    project.shots[0] = CameraShot::from_preview(
        Id(3),
        project.shots[0].span,
        CameraRigKind::Follow,
        CameraPose {
            position: [0.0, 65.6, -5.0],
            ..Default::default()
        },
        Some(&target),
    )
    .unwrap();
    project.clips[0].rate = PlaybackRate {
        numerator: 2,
        denominator: 1,
    };
    project.shots[1].span.end = MediaTime(4_000_000);
    let timeline = CompiledTimeline::new(Arc::new(project.clone())).unwrap();
    let lookups = RefCell::new(Vec::new());
    let mut frame = FramePlan::default();
    timeline
        .sample(
            MediaTime(2_500_000),
            &mut frame,
            |source, reference, time| {
                lookups.borrow_mut().push((source, reference, time));
                assert_eq!(reference, target.reference());
                Some(ActorPose {
                    position: [time.seconds(), 64.0, 0.0],
                    ..target.pose
                })
            },
        )
        .unwrap();
    assert_eq!(
        lookups.borrow().as_slice(),
        &[(Id(1), target.reference(), MediaTime(5_000_000))]
    );
    let expected = project.shots[0]
        .sample_terminal(
            |_| {
                Some(ActorPose {
                    position: [5.0, 64.0, 0.0],
                    ..target.pose
                })
            },
            None,
        )
        .unwrap();
    let incoming = project.shots[1]
        .sample(MediaTime(2_500_000), |_| None)
        .unwrap();
    assert_eq!(
        frame.base_camera,
        Some(
            project.shots[1]
                .blend_in
                .unwrap()
                .sample(expected, incoming, MediaTime(500_000))
        )
    );
}

#[test]
fn missing_outgoing_or_reused_spawn_keeps_incoming_and_missing_incoming_stays_unavailable() {
    let mut project = project();
    let target = moving_target();
    project.shots[0] = CameraShot::from_preview(
        Id(3),
        project.shots[0].span,
        CameraRigKind::Player,
        CameraPose::default(),
        Some(&target),
    )
    .unwrap();
    let expected = project.shots[1].sample(MediaTime(2_250_000), |_| None);
    let timeline = CompiledTimeline::new(Arc::new(project.clone())).unwrap();
    let mut frame = FramePlan::default();
    timeline
        .sample(MediaTime(2_250_000), &mut frame, |_, reference, _| {
            let replacement = CameraTargetRef {
                lifetime: Some(TargetLifetime {
                    spawn_revision: 3,
                    unique_id: 70,
                }),
                ..target.reference()
            };
            (reference == replacement).then_some(target.pose)
        })
        .unwrap();
    assert_eq!(frame.camera, expected);
    project.shots[0].target_lifetime = None;
    CompiledTimeline::new(Arc::new(project.clone()))
        .unwrap()
        .sample(MediaTime(2_250_000), &mut frame, |_, _, _| {
            panic!("unbound outgoing target must not be fabricated")
        })
        .unwrap();
    assert_eq!(frame.camera, expected);
    let span = project.shots[1].span;
    project.shots[1] = CameraShot::from_preview(
        Id(4),
        span,
        CameraRigKind::Player,
        CameraPose::default(),
        Some(&target),
    )
    .unwrap()
    .with_blend_in(Some(CameraBlend {
        duration: MediaTime(1_000_000),
        easing: Easing::Smooth,
    }))
    .unwrap();
    CompiledTimeline::new(Arc::new(project))
        .unwrap()
        .sample(MediaTime(2_250_000), &mut frame, |_, _, _| None)
        .unwrap();
    assert!(frame.camera.is_none());
}

#[test]
fn trim_split_move_and_undo_preserve_explicit_entry_policy() {
    let project = project();
    let incoming = &project.shots[1];
    assert!(
        incoming
            .trimmed(Span::new(MediaTime(2_250_000), incoming.span.end).unwrap())
            .unwrap()
            .blend_in
            .is_none()
    );
    assert_eq!(
        incoming
            .trimmed(Span::new(incoming.span.start, MediaTime(2_250_000)).unwrap())
            .unwrap()
            .blend_in
            .unwrap()
            .duration,
        MediaTime(250_000)
    );
    let split = MediaTime(3_000_000);
    assert_eq!(
        incoming
            .trimmed(Span::new(incoming.span.start, split).unwrap())
            .unwrap()
            .blend_in,
        incoming.blend_in
    );
    assert!(
        incoming
            .trimmed(Span::new(split, incoming.span.end).unwrap())
            .unwrap()
            .blend_in
            .is_none()
    );
    let mut moved = project.clone();
    for shot in &mut moved.shots {
        shot.span.start.0 += 1_000_000;
        shot.span.end.0 += 1_000_000;
    }
    assert_eq!(
        sample(&moved, 3_500_000).camera,
        sample(&project, 2_500_000).camera
    );
    let mut separated = project.clone();
    separated.shots[1].span.start.0 += 1_000_000;
    separated.shots[1].span.end.0 += 1_000_000;
    assert_eq!(
        sample(&separated, 3_250_000).camera,
        separated.shots[1].sample(MediaTime(3_250_000), |_| None)
    );
    let mut editor = crate::edit::Editor::new(project.clone()).unwrap();
    editor
        .apply(
            "Cut",
            &[crate::edit::Command::ReplaceShot {
                shot: incoming.with_blend_in(None).unwrap(),
            }],
        )
        .unwrap();
    assert!(editor.project().shots[1].blend_in.is_none());
    editor.undo().unwrap();
    assert_eq!(editor.project().shots[1].blend_in, incoming.blend_in);
    editor.redo().unwrap();
    assert!(editor.project().shots[1].blend_in.is_none());
}

#[test]
fn actual_camera_priority_and_late_picture_transitions_control_entry_eligibility() {
    let mut project = project();
    let high = shot(
        5,
        1_000_000,
        2_000_000,
        CameraPose {
            position: [20.0, 64.0, 0.0],
            ..Default::default()
        },
    );
    project.shots.push(high.clone());
    assert_eq!(
        sample(&project, 2_000_000).camera,
        high.sample_terminal(|_| None, None)
    );
    // A later picture dissolve invalidates the whole entry interval, including its earlier clear picture.
    project.shots.pop();
    project.clips[0].source_range.end = MediaTime(3_000_000);
    let mut next = project.clips[0].clone();
    next.id = Id(5);
    next.at = MediaTime(3_000_000);
    next.source_range = Span::new(MediaTime(3_000_000), MediaTime(8_000_000)).unwrap();
    project.clips.push(next);
    project.transitions.push(crate::model::Transition {
        id: Id(6),
        from: Id(2),
        to: Id(5),
        duration: MediaTime(1_000_000),
        kind: crate::model::TransitionKind::Crossfade,
    });
    assert_eq!(
        sample(&project, 2_250_000).camera,
        project.shots[1].sample(MediaTime(2_250_000), |_| None)
    );
    // Muted picture lanes cannot provide a source to either rig.
    project.muted_tracks.insert(crate::tracks::Track::Video(0));
    assert!(sample(&project, 2_250_000).camera.is_none());
}

#[test]
fn every_subject_rig_uses_the_current_recorded_spawn_and_quaternion_blend() {
    let target = moving_target();
    for outgoing in [
        CameraRigKind::Player,
        CameraRigKind::Track,
        CameraRigKind::Follow,
        CameraRigKind::Orbit,
    ] {
        for incoming in [
            CameraRigKind::Player,
            CameraRigKind::Track,
            CameraRigKind::Follow,
            CameraRigKind::Orbit,
        ] {
            let mut project = project();
            for (index, kind, position) in [
                (0, outgoing, [0.0, 66.0, -5.0]),
                (1, incoming, [8.0, 68.0, -2.0]),
            ] {
                let original = project.shots[index].clone();
                project.shots[index] = CameraShot::from_preview(
                    original.id,
                    original.span,
                    kind,
                    CameraPose {
                        position,
                        roll: 25.0,
                        ..Default::default()
                    },
                    Some(&target),
                )
                .unwrap();
                project.shots[index].blend_in = original.blend_in;
            }
            let timeline = CompiledTimeline::new(Arc::new(project.clone())).unwrap();
            let at = MediaTime(2_500_000);
            let current = ActorPose {
                position: [3.0, 64.0, 2.0],
                yaw: 30.0,
                pitch: 10.0,
                ..target.pose
            };
            let mut frame = FramePlan::default();
            timeline
                .sample(at, &mut frame, |source, reference, time| {
                    assert_eq!((source, reference, time), (Id(1), target.reference(), at));
                    Some(current)
                })
                .unwrap();
            let from = project.shots[0]
                .sample_terminal(|_| Some(current), None)
                .unwrap();
            let from =
                TerminalMotion::new(&project.shots[0], None).extrapolated(from, MediaTime(500_000));
            let to = project.shots[1].sample(at, |_| Some(current)).unwrap();
            assert_eq!(
                frame.base_camera,
                Some(
                    project.shots[1]
                        .blend_in
                        .unwrap()
                        .sample(from, to, MediaTime(500_000))
                )
            );
        }
    }
}

#[test]
fn body_and_hand_policy_requires_a_shared_player_pov_during_blends() {
    let mut project = project();
    let target = moving_target();
    for index in 0..2 {
        let original = project.shots[index].clone();
        project.shots[index] = CameraShot::from_preview(
            original.id,
            original.span,
            CameraRigKind::Player,
            CameraPose::default(),
            Some(&target),
        )
        .unwrap();
        project.shots[index].blend_in = original.blend_in;
        project.shots[index].hide_hand = false;
    }
    let mut frame = FramePlan::default();
    CompiledTimeline::new(Arc::new(project.clone()))
        .unwrap()
        .sample(MediaTime(2_500_000), &mut frame, |_, _, _| {
            Some(target.pose)
        })
        .unwrap();
    assert_eq!(frame.spectated, Some(target.reference()));
    assert!(!frame.hide_hand);
    project.shots[0] = shot(3, 0, 2_000_000, CameraPose::default());
    CompiledTimeline::new(Arc::new(project))
        .unwrap()
        .sample(MediaTime(2_500_000), &mut frame, |_, _, _| {
            Some(target.pose)
        })
        .unwrap();
    assert!(frame.spectated.is_none());
    assert!(frame.hide_hand);
}
