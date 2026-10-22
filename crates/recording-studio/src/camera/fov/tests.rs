use super::*;

/// One native eye and finite target lifetime suffice for every rig without any external recording.
fn target() -> CameraTarget {
    CameraTarget {
        runtime_id: 7,
        unique_id: 70,
        spawn_revision: 2,
        name: "Subject".into(),
        kind: CameraTargetKind::Player,
        entity_type: None,
        view: Default::default(),
        pose: ActorPose {
            position: [2.0, 64.0, 8.0],
            velocity: [0.0; 3],
            yaw: 35.0,
            pitch: 0.0,
            roll: 0.0,
            eye_offset: Some([0.0, 1.62, 0.0]),
        },
        teleported: false,
    }
}

/// The inspected view deliberately differs from every old fixed Follow/Orbit preset.
fn shot(kind: CameraRigKind) -> CameraShot {
    CameraShot::from_preview(
        Id(1),
        Span::new(MediaTime::ZERO, MediaTime(2_000_000)).unwrap(),
        kind,
        CameraPose {
            position: [-6.0, 70.0, 4.0],
            yaw: 45.0,
            pitch: 10.0,
            roll: 7.0,
            fov: 72.0,
        },
        Some(&target()),
    )
    .unwrap()
}

#[test]
fn tracking_keeps_world_position_while_following_keeps_its_relative_offset() {
    let original = target().pose;
    let mut moving = original;
    moving.position[0] += 10.0;
    moving.yaw += 90.0;
    let track = shot(CameraRigKind::Track);
    let first = track.sample(MediaTime::ZERO, |_| Some(original)).unwrap();
    let last = track
        .sample(MediaTime(1_000_000), |_| Some(moving))
        .unwrap();
    assert_eq!(first.position, last.position);
    assert_ne!(first.yaw, last.yaw);
    assert_eq!(last, first.looking_at(moving.centre()));
    let follow = shot(CameraRigKind::Follow);
    assert_ne!(
        follow
            .sample(MediaTime::ZERO, |_| Some(original))
            .unwrap()
            .position,
        follow
            .sample(MediaTime(1_000_000), |_| Some(moving))
            .unwrap()
            .position
    );
    assert_eq!(
        track
            .retargeted(&CameraTarget {
                spawn_revision: 3,
                ..target()
            })
            .unwrap()
            .target_lifetime
            .unwrap()
            .spawn_revision,
        3
    );
}

#[test]
fn targeted_creation_preserves_the_inspected_position_roll_and_lens() {
    for kind in [
        CameraRigKind::Track,
        CameraRigKind::Follow,
        CameraRigKind::Orbit,
    ] {
        let camera = shot(kind);
        let pose = camera
            .sample(MediaTime::ZERO, |_| Some(target().pose))
            .unwrap();
        for (actual, expected) in pose.position.into_iter().zip([-6.0, 70.0, 4.0]) {
            assert!((actual - expected).abs() < 1e-10);
        }
        assert_eq!(pose.roll, 7.0);
        assert_eq!(pose.fov, 72.0);
    }
}

#[test]
fn lens_keys_animate_every_rig_without_changing_motion_and_numeric_edits_key_the_lens_only() {
    for &kind in CameraRigKind::ALL {
        let original = shot(kind);
        let camera = original
            .with_fov_key(MediaTime::ZERO, 60.0)
            .unwrap()
            .with_fov_key(original.span.end, 120.0)
            .unwrap()
            .with_fov_easing(original.span.end, Easing::Linear)
            .unwrap();
        let at = MediaTime(1_000_000);
        let pose = camera.sample(at, |_| Some(target().pose)).unwrap();
        let base = original.sample(at, |_| Some(target().pose)).unwrap();
        assert_eq!(pose.position, base.position);
        assert_eq!(
            [pose.yaw, pose.pitch, pose.roll],
            [base.yaw, base.pitch, base.roll]
        );
        assert_eq!(pose.fov, 90.0);
        let edited = camera
            .with_parameter(at, CameraParameter::Fov, 100.0)
            .unwrap();
        assert_eq!(edited.kind(), kind);
        assert_eq!(edited.mode, camera.mode);
        assert_eq!(edited.fov_key_at(at).unwrap().value, 100.0);
    }
}

#[test]
fn nonlinear_lens_easing_retains_its_exact_phase_and_support_keys_after_a_trim() {
    let original = shot(CameraRigKind::Orbit)
        .with_fov_key(MediaTime::ZERO, 60.0)
        .unwrap()
        .with_fov_key(MediaTime(2_000_000), 120.0)
        .unwrap()
        .with_fov_easing(MediaTime(2_000_000), Easing::EaseIn)
        .unwrap();
    let trimmed = original
        .trimmed(Span::new(MediaTime(500_000), MediaTime(1_500_000)).unwrap())
        .unwrap();
    assert_eq!(trimmed.fov_offset, MediaTime(500_000));
    assert_eq!(trimmed.fov_keys().count(), 0);
    assert_eq!(trimmed.fov_curve, original.fov_curve);
    for at in [500_000, 750_000, 1_000_000, 1_250_000, 1_499_999] {
        assert_eq!(
            trimmed.fov_override(MediaTime(at)),
            original.fov_override(MediaTime(at))
        );
    }
    let edited = trimmed.with_fov_key(MediaTime(1_000_000), 80.0).unwrap();
    assert_eq!(
        edited.fov_key_at(MediaTime(1_000_000)).unwrap().at,
        MediaTime(1_000_000)
    );
}

#[test]
fn old_project_camera_fields_remain_optional_and_unknown_eye_transitions_do_not_fabricate_pov() {
    let camera = shot(CameraRigKind::Follow);
    let mut json = serde_json::to_value(&camera).unwrap();
    let object = json.as_object_mut().unwrap();
    object.remove("fov_curve");
    object.remove("fov_offset");
    object
        .get_mut("mode")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("roll");
    let old: CameraShot = serde_json::from_value(json).unwrap();
    old.validate().unwrap();
    assert!(old.fov_curve.is_none());
    assert_eq!(old.fov_offset, MediaTime::ZERO);
    assert_eq!(
        old.parameter(CameraParameter::Roll, MediaTime::ZERO),
        Some(0.0)
    );
    let known = target().pose;
    let mut unknown = known;
    unknown.eye_offset = None;
    assert_eq!(known.interpolated(unknown, 0.5).eye_offset, None);
    assert_eq!(unknown.interpolated(known, 0.5).eye_offset, None);
    assert!(
        shot(CameraRigKind::Player)
            .sample(MediaTime(1), |_| Some(unknown))
            .is_none()
    );
}

#[test]
fn timeline_lens_drag_keeps_the_rig_and_key_order_and_preserves_exact_values_without_vertical_motion()
 {
    let camera = shot(CameraRigKind::Track)
        .with_fov_key(MediaTime::ZERO, 1.0)
        .unwrap()
        .with_fov_key(MediaTime(2_000_000), 120.0)
        .unwrap();
    let mut project = crate::Project::new("lens");
    project.shots.push(camera.clone());
    let key = crate::timeline::TimelineKey {
        item: camera.id,
        kind: crate::timeline::KeyKind::CameraFov,
        at: MediaTime::ZERO,
    };
    let (command, key) = crate::timeline::move_key(&project, key, MediaTime(500_000), 0.0).unwrap();
    let crate::edit::Command::ReplaceShot { shot } = command else {
        panic!("camera edit required")
    };
    assert_eq!(shot.mode, camera.mode);
    assert_eq!(key.at, MediaTime(500_000));
    assert_eq!(shot.fov_key_at(MediaTime(500_000)).unwrap().value, 1.0);
    assert!(
        shot.with_fov_key_time(MediaTime(500_000), MediaTime(2_000_000))
            .is_err()
    );
}

#[test]
fn project_key_budget_includes_independent_lens_curves() {
    let mut project = crate::Project::new("bounded lenses");
    let count = crate::model::MAX_PROJECT_KEYS / crate::effect::MAX_CURVE_KEYS + 1;
    for index in 0..count {
        let mut camera = shot(CameraRigKind::Free);
        camera.id = Id(index as u64 + 1);
        camera.fov_curve = Some(Curve {
            keys: (0..crate::effect::MAX_CURVE_KEYS)
                .map(|index| CurveKey {
                    at: MediaTime(index as u64),
                    value: 60.0,
                    easing: Easing::Linear,
                })
                .collect(),
        });
        project.shots.push(camera);
    }
    assert!(matches!(
        project.validate(),
        Err(StudioError::Limit {
            limit: crate::model::MAX_PROJECT_KEYS,
            ..
        })
    ));
}

#[test]
fn evaluated_pov_identity_is_independent_of_hud_and_resets_outside_the_shot() {
    let mut project = crate::Project::new("POV identity");
    let mut camera = shot(CameraRigKind::Player);
    camera.hide_hud = false;
    project.export.include_recorded_hud = true;
    project.sources.push(crate::model::MediaSource {
        id: Id(2),
        name: "Replay".into(),
        path: "replay".into(),
        kind: crate::model::MediaKind::Replay,
        duration: camera.span.end,
        size: [1280, 720],
        frame_rate: None,
        sha256: None,
    });
    project.clips.push(crate::model::Clip {
        id: Id(3),
        source: Id(2),
        source_range: camera.span,
        at: MediaTime::ZERO,
        rate: crate::PlaybackRate::default(),
        speed_ramp: None,
        track: 0,
        framing: Default::default(),
        enabled: true,
    });
    project.shots.push(camera);
    let timeline = crate::playback::CompiledTimeline::new(Arc::new(project)).unwrap();
    let mut plan = crate::playback::FramePlan::default();
    timeline
        .sample(MediaTime(1_000_000), &mut plan, |_, reference, _| {
            target().matches(reference).then_some(target().pose)
        })
        .unwrap();
    assert_eq!(plan.spectated, Some(target().reference()));
    assert!(!plan.hide_hud);
    timeline
        .sample(MediaTime(2_000_000), &mut plan, |_, _, _| None)
        .unwrap();
    assert_eq!(plan.spectated, None);
}
