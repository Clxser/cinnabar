use super::*;
use crate::{
    camera::{CameraRigKind, CameraShot},
    edit::Editor,
};

/// Independent channels can be exercised without media IO or engine resources.
fn project() -> Project {
    let mut project = Project::new("Graph fixture");
    project.shots.push(
        CameraShot::from_preview(
            Id(1),
            Span::new(MediaTime::ZERO, MediaTime(2_000_000)).unwrap(),
            CameraRigKind::Free,
            CameraPose::default(),
            None,
        )
        .unwrap(),
    );
    project
}

#[test]
fn independent_channels_survive_serialization_trim_move_undo_and_out_of_order_sampling() {
    let original = project();
    let base = CameraPose::default();
    let channel = GraphChannel::Camera(CameraChannel::X);
    let mut editor = Editor::new(original.clone()).unwrap();
    let command = set_key(
        editor.project(),
        Id(1),
        channel,
        None,
        MediaTime(2_000_000),
        12.0,
        base,
    )
    .unwrap();
    editor.apply("X curve", &[command]).unwrap();
    let key = GraphKey {
        item: Id(1),
        channel,
        at: MediaTime(2_000_000),
    };
    let ease = with_handle(Easing::Smooth, GraphControl::Outgoing, [0.1, 0.6]).unwrap();
    editor
        .apply(
            "Handle",
            &[set_easing(editor.project(), key, ease, base).unwrap()],
        )
        .unwrap();
    let saved = Project::from_json(&editor.project().to_json().unwrap()).unwrap();
    assert_eq!(saved, *editor.project().as_ref());
    let shot = &saved.shots[0];
    let trimmed = shot
        .trimmed(Span::new(MediaTime(500_000), MediaTime(1_500_000)).unwrap())
        .unwrap();
    let mut moved = trimmed.clone();
    moved.span = Span::new(MediaTime(4_000_000), MediaTime(5_000_000)).unwrap();
    for at in [1_400_000, 600_000, 1_000_000, 500_000] {
        let expected = shot.sample(MediaTime(at), |_| None).unwrap();
        assert_eq!(trimmed.sample(MediaTime(at), |_| None), Some(expected));
        assert_eq!(
            moved.sample(MediaTime(at + 3_500_000), |_| None),
            Some(expected)
        );
        assert_eq!(expected.position[1], base.position[1]);
        assert_eq!(expected.yaw, base.yaw);
    }
    editor.undo().unwrap();
    editor.undo().unwrap();
    assert_eq!(*editor.project().as_ref(), original);
    editor.redo().unwrap();
    editor.redo().unwrap();
    assert_eq!(*editor.project().as_ref(), saved);
}

#[test]
fn key_moves_cannot_replace_neighbors_and_reset_restores_the_saved_rig() {
    let mut project = project();
    let channel = GraphChannel::Camera(CameraChannel::Roll);
    let base = CameraPose::default();
    if let Command::ReplaceShot { shot } = set_key(
        &project,
        Id(1),
        channel,
        None,
        MediaTime(1_000_000),
        20.0,
        base,
    )
    .unwrap()
    {
        project.shots[0] = shot;
    }
    let command = set_key(
        &project,
        Id(1),
        channel,
        Some(MediaTime::ZERO),
        MediaTime(1_000_000),
        -30.0,
        base,
    )
    .unwrap();
    if let Command::ReplaceShot { shot } = command {
        let keys = &shot.animation.curves[&CameraChannel::Roll].curve.keys;
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].at, MediaTime(999_999));
        assert_eq!(keys[1].value, 20.0);
    } else {
        panic!("camera graph must use normal shot metadata");
    }
    assert!(
        set_key(
            &project,
            Id(1),
            channel,
            None,
            MediaTime::ZERO,
            f64::NAN,
            base
        )
        .is_err()
    );
    if let Command::ReplaceShot { shot } = reset(&project, Id(1), channel).unwrap() {
        assert!(shot.animation.is_empty());
    }
}

#[test]
fn weighted_handles_transform_back_through_speed_source_clock_without_changing_other_rates() {
    use crate::{
        PlaybackRate,
        model::{Clip, Framing, MediaKind, MediaSource},
        retime::{SpeedKey, SpeedRamp},
    };
    let mut project = Project::new("Speed graph");
    project.sources.push(MediaSource {
        id: Id(1),
        name: "Anonymous".into(),
        path: "fixture.mp4".into(),
        kind: MediaKind::Video,
        duration: MediaTime(4_000_000),
        size: [1280, 720],
        frame_rate: None,
        sha256: None,
    });
    project.clips.push(Clip {
        id: Id(2),
        source: Id(1),
        source_range: Span::new(MediaTime::ZERO, MediaTime(4_000_000)).unwrap(),
        at: MediaTime(1_000_000),
        rate: PlaybackRate {
            numerator: 2,
            denominator: 1,
        },
        speed_ramp: Some(SpeedRamp {
            keys: vec![
                SpeedKey {
                    at: MediaTime::ZERO,
                    rate: PlaybackRate {
                        numerator: 1,
                        denominator: 3,
                    },
                    easing: Easing::Linear,
                },
                SpeedKey {
                    at: MediaTime(4_000_000),
                    rate: PlaybackRate {
                        numerator: 7,
                        denominator: 3,
                    },
                    easing: Easing::Smooth,
                },
            ],
        }),
        track: 0,
        framing: Framing::default(),
        enabled: true,
    });
    let graph =
        GraphCurve::new(&project, Id(2), GraphChannel::Speed, CameraPose::default()).unwrap();
    let at = graph.points[1].at;
    let handles = graph.handles(at).unwrap();
    let normalized = graph.handle_input(at, handles[0]).unwrap();
    assert!((normalized[0] - 1.0 / 3.0).abs() < 1e-5);
    assert!(normalized[1].abs() < 1e-10);
    let expected = project.clips[0].speed_ramp.as_ref().unwrap();
    let command = set_easing(
        &project,
        GraphKey {
            item: Id(2),
            channel: GraphChannel::Speed,
            at,
        },
        Easing::EaseOut,
        CameraPose::default(),
    )
    .unwrap();
    if let Command::SetSpeedRamp {
        ramp: Some(ramp), ..
    } = command
    {
        assert_eq!(ramp.keys[0].rate, expected.keys[0].rate);
        assert_eq!(ramp.keys[1].rate, expected.keys[1].rate);
    } else {
        panic!("speed graphs must preserve source retiming");
    }
}

/// Flat endpoint positions deliberately hide a Bezier excursion from raw scalar key interpolation.
fn rail_project(timing: crate::camera::PathTiming) -> Project {
    use crate::camera::{CameraHandles, CameraKey, CameraMode};
    let mut project = project();
    let pose = CameraPose {
        position: [0.0; 3],
        ..Default::default()
    };
    project.shots[0].mode = CameraMode::Path {
        keys: vec![
            CameraKey {
                at: MediaTime::ZERO,
                pose,
                easing: Easing::Linear,
                handles: Some(CameraHandles {
                    outgoing: Some([8.0, 4.0, 0.0]),
                    ..Default::default()
                }),
            },
            CameraKey {
                at: MediaTime(2_000_000),
                pose: CameraPose { yaw: 80.0, ..pose },
                easing: Easing::EaseIn,
                handles: Some(CameraHandles {
                    incoming: Some([8.0, -4.0, 0.0]),
                    ..Default::default()
                }),
            },
        ],
        smooth_position: true,
    };
    project.shots[0].path_timing = timing;
    project.validate().unwrap();
    project
}

#[test]
fn zero_delta_seed_preserves_bezier_easing_distance_and_global_rails_through_trim_and_move() {
    use crate::camera::{CompiledCameraPath, PathTiming};
    for timing in [
        PathTiming::Keyed,
        PathTiming::Distance,
        PathTiming::GlobalDistance,
    ] {
        let original = rail_project(timing);
        let untouched = Arc::new(original.clone());
        let graph = GraphCurve::new_reusing(
            Arc::clone(&untouched),
            Id(1),
            GraphChannel::Camera(CameraChannel::X),
            CameraPose::default(),
            None,
        )
        .unwrap();
        let key = GraphKey {
            item: Id(1),
            channel: graph.channel,
            at: graph.span.end,
        };
        let Command::ReplaceShot { shot } =
            set_easing_on(&graph, &untouched, key, Easing::EaseOut).unwrap()
        else {
            panic!("camera command");
        };
        let compiled = CompiledCameraPath::new(&shot);
        for at in [100_000, 500_000, 1_000_000, 1_900_000] {
            assert_eq!(
                shot.sample_compiled(MediaTime(at), |_| None, compiled.as_ref()),
                original.shots[0].sample_compiled(MediaTime(at), |_| None, compiled.as_ref()),
                "editing a zero offset's easing cannot bake or flatten the original rail"
            );
        }
        let mut project = original.clone();
        project.shots[0] = project.shots[0]
            .trimmed(Span::new(MediaTime(500_000), MediaTime(1_500_000)).unwrap())
            .unwrap();
        project.shots[0].span = Span::new(MediaTime(3_000_000), MediaTime(4_000_000)).unwrap();
        let source = Arc::new(project);
        let graph = GraphCurve::new_reusing(
            Arc::clone(&source),
            Id(1),
            GraphChannel::Camera(CameraChannel::X),
            CameraPose::default(),
            None,
        )
        .unwrap();
        assert!(
            graph.points.is_empty(),
            "support keys remain outside the retained trim"
        );
        let original_path = CompiledCameraPath::new(&original.shots[0]);
        for at in [3_000_000, 3_100_000, 3_500_000, 3_999_999] {
            let expected = original.shots[0]
                .sample_compiled(MediaTime(at - 2_500_000), |_| None, original_path.as_ref())
                .unwrap();
            assert_eq!(graph.sample(MediaTime(at)), expected.position[0]);
            assert!(
                graph.range[0] <= expected.position[0] && graph.range[1] >= expected.position[0]
            );
        }
        let at = MediaTime(3_500_000);
        let expected = graph.sample(at);
        let Command::ReplaceShot { shot } =
            set_key_on(&graph, &source, None, at, expected).unwrap()
        else {
            panic!("camera command");
        };
        let channel = &shot.animation.curves[&CameraChannel::X];
        assert!(channel.additive);
        assert_eq!(channel.offset, MediaTime(500_000));
        assert_eq!(
            channel.curve.keys.len(),
            3,
            "both original support keys are retained"
        );
        assert!(channel.curve.keys.iter().all(|key| key.value == 0.0));
        let path = CompiledCameraPath::new(&shot);
        for at in [3_000_000, 3_100_000, 3_500_000, 3_999_999] {
            assert_eq!(
                shot.sample_compiled(MediaTime(at), |_| None, path.as_ref()),
                source.shots[0].sample_compiled(MediaTime(at), |_| None, path.as_ref())
            );
        }
        let Command::ReplaceShot { shot } =
            set_key_on(&graph, &source, None, at, expected + 3.0).unwrap()
        else {
            panic!("camera command");
        };
        assert_eq!(
            shot.sample_compiled(at, |_| None, path.as_ref())
                .unwrap()
                .position[0],
            expected + 3.0
        );
    }
}

#[test]
fn maximum_rail_frozen_drags_and_animation_documents_reuse_quadrature_and_release_old_documents() {
    use crate::camera::{CameraKey, CameraMode, CompiledCameraPath, MAX_CAMERA_KEYS, PathTiming};
    let mut project = rail_project(PathTiming::Distance);
    project.shots[0].span.end = MediaTime((MAX_CAMERA_KEYS as u64 - 1) * 1_000);
    project.shots[0].mode = CameraMode::Path {
        keys: (0..MAX_CAMERA_KEYS)
            .map(|index| CameraKey {
                at: MediaTime(index as u64 * 1_000),
                pose: CameraPose {
                    position: [index as f64, (index % 7) as f64, 0.0],
                    ..Default::default()
                },
                easing: Easing::Smooth,
                handles: None,
            })
            .collect(),
        smooth_position: true,
    };
    let source = Arc::new(project);
    let weak_source = Arc::downgrade(&source);
    let graph = GraphCurve::new_reusing(
        Arc::clone(&source),
        Id(1),
        GraphChannel::Camera(CameraChannel::X),
        CameraPose::default(),
        None,
    )
    .unwrap();
    assert_eq!(graph.points.len(), MAX_CAMERA_KEYS);
    let cells = CompiledCameraPath::integrated_cells();
    let mut last = None;
    for offset in 1..=8 {
        let command = set_key_on(
            &graph,
            &source,
            Some(MediaTime(4_000)),
            MediaTime(4_000),
            4.0 + f64::from(offset),
        )
        .unwrap();
        last = Some(command);
        assert_eq!(
            CompiledCameraPath::integrated_cells(),
            cells,
            "pointer drags do not integrate a rail"
        );
    }
    let mut edited = source.as_ref().clone();
    let Command::ReplaceShot { shot } = last.unwrap() else {
        panic!("camera command");
    };
    assert_eq!(
        shot.animation.curves[&CameraChannel::X].curve.keys.len(),
        MAX_CAMERA_KEYS
    );
    edited.shots[0] = shot;
    let edited = Arc::new(edited);
    let next = GraphCurve::new_reusing(
        Arc::clone(&edited),
        Id(1),
        graph.channel,
        CameraPose::default(),
        Some(&graph),
    )
    .unwrap();
    assert_eq!(CompiledCameraPath::integrated_cells(), cells);
    assert!(Arc::ptr_eq(
        graph.base.as_ref().unwrap(),
        next.base.as_ref().unwrap()
    ));
    assert_eq!(next.sample(MediaTime(4_000)), 12.0);
    assert!(
        set_key_on(&graph, &edited, None, MediaTime(4_000), 1.0).is_err(),
        "a stale snapshot cannot overwrite a newer document"
    );
    let wrong = GraphKey {
        item: Id(99),
        channel: graph.channel,
        at: MediaTime(4_000),
    };
    assert!(set_easing_on(&graph, &source, wrong, Easing::Linear).is_err());
    drop(graph);
    drop(source);
    assert!(
        weak_source.upgrade().is_none(),
        "geometry reuse does not pin an old project"
    );
}

#[test]
fn support_only_trimmed_curves_and_opposite_finite_extremes_have_usable_ranges() {
    use crate::camera::CameraChannelCurve;
    let mut project = project();
    project.shots[0].animation.curves.insert(
        CameraChannel::X,
        CameraChannelCurve {
            curve: Curve {
                keys: vec![
                    CurveKey {
                        at: MediaTime::ZERO,
                        value: 0.0,
                        easing: Easing::Linear,
                    },
                    CurveKey {
                        at: MediaTime(2_000_000),
                        value: 20.0,
                        easing: Easing::Linear,
                    },
                ],
            },
            offset: MediaTime::ZERO,
            additive: false,
        },
    );
    project.shots[0] = project.shots[0]
        .trimmed(Span::new(MediaTime(500_000), MediaTime(1_500_000)).unwrap())
        .unwrap();
    let graph = GraphCurve::new(
        &project,
        Id(1),
        GraphChannel::Camera(CameraChannel::X),
        CameraPose::default(),
    )
    .unwrap();
    assert!(graph.points.is_empty());
    assert_eq!(graph.sample(graph.span.start), 5.0);
    assert_eq!(graph.sample(graph.span.end), 15.0);
    assert!(graph.range[0] < 5.0 && graph.range[1] > 15.0);
    let channel = project.shots[0]
        .animation
        .curves
        .get_mut(&CameraChannel::X)
        .unwrap();
    channel.curve.keys[0].value = -f64::MAX;
    channel.curve.keys[1].value = f64::MAX;
    project.validate().unwrap();
    let graph = GraphCurve::new(
        &project,
        Id(1),
        GraphChannel::Camera(CameraChannel::X),
        CameraPose::default(),
    )
    .unwrap();
    assert!(graph.range.into_iter().all(f64::is_finite));
    assert_eq!(graph.sample(MediaTime(1_000_000)), 0.0);
    assert_eq!(graph.range_fraction(0.0), 0.5);
    assert_eq!(graph.range_value(0.5), 0.0);
    for n in 0..=100 {
        let value = graph.range_value(f64::from(n) / 100.0);
        assert!(value.is_finite());
        assert!(graph.range_fraction(value).is_finite());
    }
    let encoded = serde_json::to_value(&project.shots[0].animation).unwrap();
    assert!(encoded["curves"]["x"].get("additive").is_none());
    let decoded: crate::camera::CameraAnimation = serde_json::from_value(encoded).unwrap();
    assert!(
        !decoded.curves[&CameraChannel::X].additive,
        "legacy absolute metadata retains its meaning"
    );
}

#[test]
fn additive_pitch_and_subject_rigs_use_offsets_without_fabricated_historical_poses() {
    use crate::camera::{ActorPose, CameraMode};
    let mut free = project();
    free.shots[0].mode = CameraMode::Free {
        pose: CameraPose {
            pitch: 80.0,
            ..Default::default()
        },
    };
    let Command::ReplaceShot { shot } = set_key(
        &free,
        Id(1),
        GraphChannel::Camera(CameraChannel::Pitch),
        None,
        MediaTime(1_000_000),
        -90.0,
        CameraPose::default(),
    )
    .unwrap() else {
        panic!("camera command");
    };
    assert_eq!(
        shot.animation.curves[&CameraChannel::Pitch].curve.keys[1].value,
        -170.0
    );
    assert_eq!(
        shot.sample(MediaTime(1_000_000), |_| None).unwrap().pitch,
        -90.0
    );
    let decoded: crate::camera::CameraShot =
        serde_json::from_slice(&serde_json::to_vec(&shot).unwrap()).unwrap();
    assert_eq!(decoded, shot);
    let mut project = project();
    project.shots[0].mode = CameraMode::Player {
        actor: 7,
        fov: 70.0,
    };
    let source = Arc::new(project);
    let graph = GraphCurve::new_reusing(
        Arc::clone(&source),
        Id(1),
        GraphChannel::Camera(CameraChannel::Pitch),
        CameraPose {
            pitch: 40.0,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    assert!(graph.relative_values());
    assert_eq!(graph.sample(MediaTime(1_000_000)), 0.0);
    let Command::ReplaceShot { shot } =
        set_key_on(&graph, &source, None, MediaTime(1_000_000), 180.0).unwrap()
    else {
        panic!("camera command");
    };
    let actor = ActorPose {
        position: [0.0; 3],
        velocity: [0.0; 3],
        yaw: 30.0,
        pitch: -80.0,
        roll: 0.0,
        eye_offset: Some([0.0, 1.6, 0.0]),
    };
    let pose = shot.sample(MediaTime(1_000_000), |_| Some(actor)).unwrap();
    assert_eq!(pose.pitch, 90.0);
    pose.validate().unwrap();
}

#[test]
fn graph_speed_quantization_remains_inside_combined_rate_limits_at_both_edges() {
    use crate::{
        PlaybackRate,
        model::{Clip, Framing, MediaKind, MediaSource},
        retime::{SpeedKey, SpeedRamp},
    };
    for base in [
        PlaybackRate {
            numerator: 3,
            denominator: 2,
        },
        PlaybackRate {
            numerator: 3,
            denominator: 7,
        },
    ] {
        let mut project = Project::new("Rational boundary");
        project.sources.push(MediaSource {
            id: Id(1),
            name: "Fixture".into(),
            path: "fixture.mp4".into(),
            kind: MediaKind::Video,
            duration: MediaTime(4_000_000),
            size: [1280, 720],
            frame_rate: None,
            sha256: None,
        });
        project.clips.push(Clip {
            id: Id(2),
            source: Id(1),
            source_range: Span::new(MediaTime::ZERO, MediaTime(4_000_000)).unwrap(),
            at: MediaTime::ZERO,
            rate: base,
            speed_ramp: Some(SpeedRamp {
                keys: vec![
                    SpeedKey {
                        at: MediaTime::ZERO,
                        rate: PlaybackRate::default(),
                        easing: Easing::Linear,
                    },
                    SpeedKey {
                        at: MediaTime(4_000_000),
                        rate: PlaybackRate {
                            numerator: 2,
                            denominator: 1,
                        },
                        easing: Easing::Linear,
                    },
                ],
            }),
            track: 0,
            framing: Framing::default(),
            enabled: true,
        });
        let source = Arc::new(project);
        let graph = GraphCurve::new_reusing(
            Arc::clone(&source),
            Id(2),
            GraphChannel::Speed,
            CameraPose::default(),
            None,
        )
        .unwrap();
        let at = graph.points[1].at;
        for value in [-f64::MAX, f64::MAX] {
            let Command::SetSpeedRamp {
                ramp: Some(ramp), ..
            } = set_key_on(&graph, &source, Some(at), at, value).unwrap()
            else {
                panic!("speed command");
            };
            ramp.validate(base).unwrap();
            let rate = ramp.keys[1].rate;
            let combined = f64::from(base.numerator) * f64::from(rate.numerator)
                / f64::from(base.denominator)
                / f64::from(rate.denominator);
            let maximum = f64::from(crate::time::MAX_PLAYBACK_RATE);
            assert!((1.0 / maximum..=maximum).contains(&combined));
        }
    }
}
