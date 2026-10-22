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
