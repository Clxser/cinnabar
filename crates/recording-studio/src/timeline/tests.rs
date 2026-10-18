use super::*;
use crate::{
    PlaybackRate,
    edit::Editor,
    effect::{AutomationOrigin, Curve, CurveKey, Easing, EffectClip, EffectKind},
    model::{
        AudioClip, AudioRole, Clip, Confidence, Framing, Marker, MarkerKind, MediaKind, MediaSource,
    },
    tracks::Track,
};
use std::{collections::BTreeSet, sync::Arc};

/// Source-backed fixtures exercise complete Editor admission without media files or render carriers.
fn fixture() -> Project {
    let mut project = Project::new("timeline");
    project.sources.push(MediaSource {
        id: Id(1),
        name: "Take".into(),
        path: "take.mp4".into(),
        kind: MediaKind::Video,
        duration: MediaTime(30_000_000),
        size: [64, 64],
        frame_rate: None,
        sha256: None,
    });
    project.clips = vec![clip(2, 1_000_000), clip(3, 3_000_000)];
    project
}

/// Uniform one-second clips have enough source handles for ordinary trim and move tests.
fn clip(id: u64, at: u64) -> Clip {
    Clip {
        id: Id(id),
        source: Id(1),
        source_range: Span::new(MediaTime::ZERO, MediaTime(1_000_000)).unwrap(),
        at: MediaTime(at),
        rate: PlaybackRate::default(),
        speed_ramp: None,
        track: 0,
        framing: Framing::default(),
        enabled: true,
    }
}

/// Linked audio and generated evidence retain their parent source timestamp through whole transactions.
fn dependencies(project: &mut Project) {
    project.audio.push(AudioClip {
        id: Id(4),
        source: Id(1),
        linked_clip: Some(Id(2)),
        source_range: project.clips[0].source_range,
        at: project.clips[0].at,
        rate: PlaybackRate::default(),
        speed_ramp: None,
        role: AudioRole::Game,
        gain: 1.0,
        fade_in: MediaTime::ZERO,
        fade_out: MediaTime::ZERO,
        muted: false,
    });
    let origin = AutomationOrigin {
        source: Id(1),
        event_sequence: 1,
        template_revision: 1,
        clip_id: Some(Id(2)),
        source_time: Some(MediaTime(500_000)),
    };
    project.effects.push(EffectClip {
        id: Id(5),
        span: Span::new(MediaTime(1_250_000), MediaTime(1_750_000)).unwrap(),
        kind: EffectKind::Zoom,
        amount: Curve::constant(1.0),
        enabled: true,
        origin: Some(origin),
    });
    project.markers.push(Marker {
        id: Id(6),
        at: MediaTime(1_500_000),
        kind: MarkerKind::Hit,
        label: "Hit".into(),
        confidence: Confidence::Observed,
        origin: Some(origin),
    });
}

#[test]
fn group_move_clamps_once_preserves_spacing_and_undoes_as_one_transaction() {
    let mut project = fixture();
    dependencies(&mut project);
    let original = project.clone();
    let mut editor = Editor::new(project).unwrap();
    let selected = BTreeSet::from([Id(2), Id(3), Id(4), Id(5)]);
    let (commands, delta) = move_selection(editor.project(), &selected, -9_000_000).unwrap();
    assert_eq!(delta, -1_000_000);
    assert_eq!(commands.len(), 2);
    editor.apply("Move group", &commands).unwrap();
    let moved = editor.project();
    assert_eq!(moved.clips[0].at, MediaTime::ZERO);
    assert_eq!(moved.clips[1].at, MediaTime(2_000_000));
    assert_eq!(moved.audio[0].at, moved.clips[0].at);
    assert_eq!(moved.markers[0].at, MediaTime(500_000));
    editor.undo().unwrap();
    assert_eq!(**editor.project(), original);
    assert!(editor.undo_label().is_none());
}

#[test]
fn duplicate_remaps_linked_audio_and_generated_anchors_and_skips_occupied_space() {
    let mut project = fixture();
    dependencies(&mut project);
    project.clips.push(clip(7, 2_000_000));
    let mut editor = Editor::new(project).unwrap();
    let selected = BTreeSet::from([Id(2)]);
    let (commands, selection) = duplicate_selection(editor.project(), &selected).unwrap();
    editor.apply("Duplicate", &commands).unwrap();
    let id = *selection.first().unwrap();
    let copied = editor
        .project()
        .clips
        .iter()
        .find(|clip| clip.id == id)
        .unwrap();
    assert_eq!(copied.at, MediaTime(4_000_000));
    let audio = editor
        .project()
        .audio
        .iter()
        .find(|audio| audio.linked_clip == Some(id))
        .unwrap();
    assert_eq!(audio.at, copied.at);
    let effect = editor
        .project()
        .effects
        .iter()
        .find(|effect| effect.origin.and_then(|origin| origin.clip_id) == Some(id))
        .unwrap();
    assert_eq!(effect.span.start, MediaTime(4_250_000));
    let marker = editor
        .project()
        .markers
        .iter()
        .find(|marker| marker.origin.and_then(|origin| origin.clip_id) == Some(id))
        .unwrap();
    assert_eq!(marker.at, MediaTime(4_500_000));
}

#[test]
fn deleting_selected_parent_and_children_cleans_dependencies_without_double_delete() {
    let mut project = fixture();
    dependencies(&mut project);
    let mut editor = Editor::new(project).unwrap();
    let commands =
        delete_selection(editor.project(), &BTreeSet::from([Id(2), Id(4), Id(5)])).unwrap();
    assert_eq!(commands.len(), 1);
    editor.apply("Delete", &commands).unwrap();
    assert!(editor.project().audio.is_empty());
    assert!(editor.project().effects.is_empty());
    assert!(editor.project().markers.is_empty());
}

#[test]
fn locked_dependent_track_rejects_whole_parent_move_without_history_or_revision_changes() {
    let mut project = fixture();
    dependencies(&mut project);
    let mut editor = Editor::new(project).unwrap();
    let before = Arc::clone(editor.project());
    let revision = editor.revision();
    let (commands, _) = move_selection(&before, &BTreeSet::from([Id(2)]), 100_000).unwrap();
    assert!(
        editor
            .apply_guarded("Move", &commands, |a, b| ensure_unlocked(
                a,
                b,
                &BTreeSet::from([Track::Audio])
            ))
            .is_err()
    );
    assert!(Arc::ptr_eq(editor.project(), &before));
    assert_eq!(editor.revision(), revision);
    assert!(editor.undo_label().is_none());
}

#[test]
fn magnetic_edges_snap_to_markers_playhead_and_other_clip_edges_but_not_the_selection() {
    let mut project = fixture();
    dependencies(&mut project);
    let selected = BTreeSet::from([Id(2)]);
    let snap = SnapIndex::new(&project, &selected, MediaTime(2_500_000));
    assert_eq!(
        snap.delta(&[MediaTime(1_000_000)], 1_495_000, MediaTime(10_000)),
        (1_500_000, Some(MediaTime(2_500_000)))
    );
    assert_eq!(
        snap.delta(&[MediaTime(2_000_000)], 995_000, MediaTime(10_000)),
        (1_000_000, Some(MediaTime(3_000_000)))
    );
    assert_eq!(
        snap.delta(&[MediaTime(1_000_000)], 5_000, MediaTime(10_000)),
        (5_000, None)
    );
}

#[test]
fn effect_key_drag_preserves_order_easing_and_value_constraints() {
    let mut project = fixture();
    project.effects.push(EffectClip {
        id: Id(4),
        span: Span::new(MediaTime(1_000_000), MediaTime(2_000_000)).unwrap(),
        kind: EffectKind::Fade,
        amount: Curve {
            keys: vec![
                CurveKey {
                    at: MediaTime::ZERO,
                    value: 0.0,
                    easing: Easing::Linear,
                },
                CurveKey {
                    at: MediaTime(500_000),
                    value: 0.5,
                    easing: Easing::Smooth,
                },
                CurveKey {
                    at: MediaTime(1_000_000),
                    value: 0.0,
                    easing: Easing::Hold,
                },
            ],
        },
        enabled: true,
        origin: None,
    });
    let key = TimelineKey {
        item: Id(4),
        kind: KeyKind::Effect,
        at: MediaTime(500_000),
    };
    let (command, moved) = move_key(&project, key, MediaTime(8_000_000), 10.0).unwrap();
    assert_eq!(moved.at, MediaTime(999_999));
    let mut editor = Editor::new(project).unwrap();
    editor.apply("Move key", &[command]).unwrap();
    let sample = &editor.project().effects[0].amount.keys[1];
    assert_eq!(sample.easing, Easing::Smooth);
    assert_eq!(sample.value, 1.0);
}

#[test]
fn track_mute_roundtrips_and_filters_visuals_linked_audio_and_generated_effects() {
    let mut project = fixture();
    dependencies(&mut project);
    project.muted_tracks.insert(Track::Video(0));
    project.shots.push(
        crate::camera::CameraShot::from_preview(
            Id(7),
            Span::new(MediaTime(1_000_000), MediaTime(2_000_000)).unwrap(),
            crate::camera::CameraRigKind::Free,
            crate::camera::CameraPose::default(),
            None,
        )
        .unwrap(),
    );
    let project = Project::from_json(&project.to_json().unwrap()).unwrap();
    assert!(project.audio_muted(&project.audio[0]));
    let timeline = crate::playback::CompiledTimeline::new(Arc::new(project)).unwrap();
    let mut plan = crate::playback::FramePlan::default();
    timeline
        .sample(MediaTime(1_500_000), &mut plan, |_, _, _| None)
        .unwrap();
    assert!(plan.visuals.is_empty());
    assert!(plan.audio.is_empty());
    assert!(plan.effects.neutral());
    assert!(
        plan.shot.is_none(),
        "muting the active visual does not require an unavailable replay source"
    );
}

#[test]
fn legacy_projects_default_to_unmuted_tracks() {
    let mut value = serde_json::to_value(fixture()).unwrap();
    value.as_object_mut().unwrap().remove("muted_tracks");
    let project = Project::from_json(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(project.muted_tracks.is_empty());
}

#[test]
fn speed_value_drag_retains_exact_source_anchor_and_horizontal_drag_retains_exact_rate() {
    let mut project = fixture();
    project.clips[0].speed_ramp = Some(crate::retime::SpeedRamp {
        keys: vec![
            crate::retime::SpeedKey {
                at: MediaTime::ZERO,
                rate: PlaybackRate {
                    numerator: 1,
                    denominator: 3,
                },
                easing: Easing::Linear,
            },
            crate::retime::SpeedKey {
                at: MediaTime(500_000),
                rate: PlaybackRate::default(),
                easing: Easing::Smooth,
            },
            crate::retime::SpeedKey {
                at: MediaTime(1_000_000),
                rate: PlaybackRate::default(),
                easing: Easing::Hold,
            },
        ],
    });
    let key = TimelineKey {
        item: Id(2),
        kind: KeyKind::Speed,
        at: MediaTime::ZERO,
    };
    let at = key_time(&project, key).unwrap();
    let (_, moved) = move_key(&project, key, at, 0.1).unwrap();
    assert_eq!(moved.at, key.at);
    let (command, moved) = move_key(&project, key, MediaTime(at.0 + 10_000), 0.0).unwrap();
    assert!(moved.at > key.at);
    let Command::SetSpeedRamp {
        ramp: Some(ramp), ..
    } = command
    else {
        panic!("speed edit");
    };
    assert_eq!(
        ramp.keys[0].rate,
        PlaybackRate {
            numerator: 1,
            denominator: 3
        }
    );
    assert_eq!(ramp.keys[0].easing, Easing::Linear);
}
