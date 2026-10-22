use super::*;

/// A finite recorded identity exercises tracking presets without consulting a live actor store.
fn subject() -> CameraTarget {
    CameraTarget {
        runtime_id: 7,
        unique_id: 70,
        spawn_revision: 2,
        name: "Subject".into(),
        kind: CameraTargetKind::Player,
        entity_type: None,
        teleported: false,
        pose: ActorPose {
            position: [0.0, 64.0, 8.0],
            velocity: [0.0; 3],
            yaw: 30.0,
            pitch: 0.0,
            roll: 0.0,
            eye_offset: Some([0.0, 1.62, 0.0]),
        },
    }
}

#[test]
fn every_preset_produces_editable_valid_finite_shots_with_exact_subject_identity() {
    let subject = subject();
    let span = Span::new(MediaTime::ZERO, MediaTime(2_000_000)).unwrap();
    let pose = CameraPose {
        position: [-4.0, 67.0, -6.0],
        ..Default::default()
    };
    for &preset in CameraPreset::ALL {
        let shot = CameraShot::cinematic_preset(Id(1), span, preset, pose, Some(&subject)).unwrap();
        shot.validate().unwrap();
        for time in [MediaTime::ZERO, MediaTime(1_000_000), MediaTime(1_999_999)] {
            shot.sample(time, |reference| {
                assert_eq!(reference, subject.reference());
                Some(subject.pose)
            })
            .unwrap()
            .validate()
            .unwrap();
        }
        if preset.needs_subject() || preset == CameraPreset::Arc {
            assert_eq!(shot.target_lifetime, subject.reference().lifetime);
        } else {
            assert_eq!(shot.sample(MediaTime::ZERO, |_| None).unwrap(), pose);
        }
    }
}

#[test]
fn missing_subjects_disable_only_presets_which_need_recorded_identity() {
    let span = Span::new(MediaTime::ZERO, MediaTime(2_000_000)).unwrap();
    for &preset in CameraPreset::ALL {
        let result = CameraShot::cinematic_preset(Id(1), span, preset, CameraPose::default(), None);
        assert_eq!(result.is_err(), preset.needs_subject());
    }
}
