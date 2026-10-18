use super::*;
use crate::{
    camera::{CameraRigKind, CameraShot},
    model::{Clip, ExportSettings, MediaKind, MediaSource, TitleClip, TitleStyle},
};

/// A replay and a project title share one frame; HUD policy cannot remove either timeline item.
fn project() -> Project {
    let mut project = Project::new("HUD policy");
    let span = Span::new(MediaTime::ZERO, MediaTime(1_000_000)).unwrap();
    project.sources.push(MediaSource {
        id: Id(1),
        name: "Replay".into(),
        path: "replay".into(),
        kind: MediaKind::Replay,
        duration: span.end,
        size: [1280, 720],
        frame_rate: None,
        sha256: None,
    });
    project.clips.push(Clip {
        id: Id(2),
        source: Id(1),
        source_range: span,
        at: MediaTime::ZERO,
        rate: PlaybackRate::default(),
        speed_ramp: None,
        track: 0,
        framing: Framing::default(),
        enabled: true,
    });
    project.titles.push(TitleClip {
        id: Id(3),
        span,
        text: "Title".into(),
        anchor: [0.5, 0.5],
        size: 1.0,
        color: [1.0; 4],
        style: TitleStyle::Body,
        shadow: false,
        fade_in: MediaTime::ZERO,
        fade_out: MediaTime::ZERO,
    });
    project
}

/// Sampling uses the same project policy for preview and export.
fn sample(project: Project) -> FramePlan {
    let mut frame = FramePlan::default();
    CompiledTimeline::new(Arc::new(project))
        .unwrap()
        .sample(MediaTime::ZERO, &mut frame, |_, _, _| None)
        .unwrap();
    frame
}

#[test]
fn recorded_hud_defaults_hidden_without_removing_replay_or_titles() {
    let frame = sample(project());
    assert!(frame.hide_hud);
    assert_eq!(frame.visuals.len(), 1);
    assert_eq!(frame.titles.len(), 1);
    let mut old = serde_json::to_value(ExportSettings::default()).unwrap();
    old.as_object_mut().unwrap().remove("include_recorded_hud");
    assert!(
        !serde_json::from_value::<ExportSettings>(old)
            .unwrap()
            .include_recorded_hud
    );
}

#[test]
fn enabling_hud_remains_subject_to_the_authored_shots_hide_flag() {
    let mut project = project();
    project.export.include_recorded_hud = true;
    assert!(!sample(project.clone()).hide_hud);
    let mut shot = CameraShot::from_preview(
        Id(4),
        Span::new(MediaTime::ZERO, MediaTime(1_000_000)).unwrap(),
        CameraRigKind::Free,
        CameraPose::default(),
        None,
    )
    .unwrap();
    shot.hide_hud = true;
    project.shots.push(shot);
    assert!(sample(project.clone()).hide_hud);
    project.export.include_recorded_hud = false;
    project.shots[0].hide_hud = false;
    assert!(sample(project).hide_hud);
}
