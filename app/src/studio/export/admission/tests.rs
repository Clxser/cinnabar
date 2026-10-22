use super::*;
use recording_studio::{
    PlaybackRate,
    model::{Clip, Framing, MediaSource, Transition},
};

/// Adjacent native and raster clips require a device only where native pixels actually contribute.
fn project() -> Project {
    let mut project = Project::new("range device admission");
    for (id, kind) in [(1, MediaKind::Replay), (2, MediaKind::Video)] {
        project.sources.push(MediaSource {
            id: Id(id),
            name: "source".into(),
            path: "unused".into(),
            kind,
            duration: MediaTime(1_000_000),
            size: [16, 16],
            frame_rate: None,
            sha256: None,
        });
        project.clips.push(Clip {
            id: Id(id + 2),
            source: Id(id),
            source_range: Span::new(MediaTime::ZERO, MediaTime(1_000_000)).unwrap(),
            at: MediaTime((id - 1) * 1_000_000),
            rate: PlaybackRate::default(),
            speed_ramp: None,
            track: 0,
            framing: Framing::default(),
            enabled: true,
        });
    }
    project
}

#[test]
fn native_content_outside_the_export_range_does_not_require_a_graphics_device() {
    let mut project = project();
    project.export.range = Some(Span::new(MediaTime(1_000_000), MediaTime(2_000_000)).unwrap());
    assert!(prepare(&project, false).is_ok());
    project.export.range = None;
    assert!(prepare(&project, false).is_err());
}

#[test]
fn native_dissolve_handle_inside_the_range_still_requires_its_device() {
    let mut project = project();
    project.transitions.push(Transition {
        id: Id(5),
        from: Id(3),
        to: Id(4),
        duration: MediaTime(200_000),
        kind: TransitionKind::Crossfade,
    });
    project.export.range = Some(Span::new(MediaTime(1_000_000), MediaTime(1_050_000)).unwrap());
    assert!(prepare(&project, false).is_err());
    assert!(prepare(&project, true).is_ok());
}

/// A saved 30 FPS In remains editable after switching the output grid to 24 FPS.
fn between_frames() -> Project {
    let mut project = project();
    project.export.fps = recording_studio::FrameRate {
        numerator: 24,
        denominator: 1,
    };
    project.export.range = Some(Span::new(MediaTime(33_333), MediaTime(100_000)).unwrap());
    project.clips[0].source_range.end = MediaTime(40_000);
    project.clips[1].at = MediaTime(40_000);
    project
}

/// Deserialization exercises backward-compatible camera defaults as the shot model evolves.
fn shot(span: Span) -> recording_studio::camera::CameraShot {
    serde_json::from_value(serde_json::json!({
        "id": 8,
        "name": "Free camera",
        "span": span,
        "mode": { "kind": "free", "pose": recording_studio::camera::CameraPose::default() },
        "hide_hud": false,
        "hide_hand": false
    }))
    .unwrap()
}

#[test]
fn unsampled_native_effect_title_and_transition_prefixes_do_not_require_a_device() {
    use recording_studio::{
        effect::{Curve, EffectClip, EffectKind},
        model::{TitleClip, TitleStyle},
    };
    let mut project = between_frames();
    let prefix = Span::new(MediaTime(33_333), MediaTime(40_000)).unwrap();
    project.effects.push(EffectClip {
        id: Id(6),
        span: prefix,
        kind: EffectKind::Flash,
        amount: Curve::constant(1.0),
        enabled: true,
        origin: None,
    });
    project.titles.push(TitleClip {
        id: Id(7),
        span: prefix,
        text: "Unsampled".into(),
        anchor: [0.5, 0.5],
        size: 0.05,
        color: [1.0; 4],
        style: TitleStyle::Body,
        shadow: false,
        fade_in: MediaTime::ZERO,
        fade_out: MediaTime::ZERO,
    });
    project.transitions.push(Transition {
        id: Id(5),
        from: Id(3),
        to: Id(4),
        duration: MediaTime(2_000),
        kind: TransitionKind::DipToBlack,
    });
    project.validate().unwrap();
    assert!(prepare(&project, false).is_ok());
    project.effects[0].span.end = MediaTime(42_000);
    assert!(prepare(&project, false).is_err());
    project.effects.clear();
    project.titles[0].span.end = MediaTime(42_000);
    assert!(prepare(&project, false).is_err());
    project.titles.clear();
    project.transitions[0].duration = MediaTime(4_000);
    assert!(prepare(&project, false).is_err());
}

#[test]
fn unsampled_camera_shots_are_admitted_but_a_sampled_camera_without_native_pixels_fails() {
    let mut project = between_frames();
    project.shots.push(shot(
        Span::new(MediaTime(33_333), MediaTime(40_000)).unwrap(),
    ));
    project.validate().unwrap();
    assert!(prepare(&project, false).is_ok());
    project.shots[0].span.end = MediaTime(42_000);
    assert!(prepare(&project, true).is_err());
}

#[test]
fn camera_gap_checks_only_require_native_coverage_at_exported_timestamps() {
    let mut project = between_frames();
    project.sources[1].kind = MediaKind::Replay;
    project.clips[1].at = MediaTime(41_666);
    project.shots.push(shot(
        Span::new(MediaTime(33_333), MediaTime(100_000)).unwrap(),
    ));
    project.validate().unwrap();
    assert!(prepare(&project, true).is_ok());
    project.clips[1].at = MediaTime(41_667);
    assert!(prepare(&project, true).is_err());
}
