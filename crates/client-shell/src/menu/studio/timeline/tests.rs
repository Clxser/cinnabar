use super::*;
use launcher::menu::{MenuAction, studio::timeline::TimelineAction};

/// Pure metadata projects keep pointer regression tests independent of filesystem and GPU fixtures.
fn studio() -> Studio {
    let mut project = Project::new("timeline");
    project.sources.push(MediaSource {
        id: Id(1),
        name: "Take".into(),
        path: "take.mp4".into(),
        kind: MediaKind::Video,
        duration: MediaTime(10_000_000),
        size: [64, 64],
        frame_rate: None,
        sha256: None,
    });
    for (id, at) in [(2, 1_000_000), (3, 3_000_000)] {
        project.clips.push(Clip {
            id: Id(id),
            source: Id(1),
            source_range: Span::new(MediaTime::ZERO, MediaTime(1_000_000)).unwrap(),
            at: MediaTime(at),
            rate: PlaybackRate::default(),
            speed_ramp: None,
            track: 0,
            framing: Framing::default(),
            enabled: true,
        });
    }
    let mut studio = Studio::new("unused".into());
    studio.install(Editor::new(project).unwrap(), None);
    studio.view = Arc::new(launcher::menu::studio::Snapshot {
        visible_duration: MediaTime(10_000_000),
        ..(*studio.view).clone()
    });
    studio
}

/// Starts one native clip move with the same geometry supplied by the painted hit region.
fn begin(studio: &mut Studio) {
    assert!(studio.timeline_pointer(
        Some(MenuAction::Studio(Action::DragItem {
            id: Id(2),
            mode: DragMode::Move,
            left: 0,
            width: 1000
        })),
        Some([100.0, 10.0]),
        true,
        true,
        [false; 3]
    ));
}

#[test]
fn returning_a_drag_to_its_origin_is_a_noop_and_stationary_input_does_no_duplicate_work() {
    let mut studio = studio();
    begin(&mut studio);
    studio.timeline_pointer(None, Some([120.0, 10.0]), true, false, [false; 3]);
    let view = Arc::clone(&studio.view);
    studio.timeline_pointer(None, Some([120.0, 10.0]), true, false, [false; 3]);
    assert!(Arc::ptr_eq(&view, &studio.view));
    studio.timeline_pointer(None, Some([100.0, 10.0]), true, false, [false; 3]);
    studio.timeline_pointer(None, None, false, false, [false; 3]);
    assert_eq!(studio.project().unwrap().clips[0].at, MediaTime(1_000_000));
    assert!(studio.editor.as_ref().unwrap().undo_label().is_none());
}

#[test]
fn a_revision_change_cancels_a_drag_even_if_undo_restores_the_same_project_arc() {
    let mut studio = studio();
    let original = Arc::clone(studio.project().unwrap());
    begin(&mut studio);
    studio.timeline_pointer(None, Some([120.0, 10.0]), true, false, [false; 3]);
    let editor = studio.editor.as_mut().unwrap();
    editor
        .apply(
            "Rename",
            &[Command::Rename {
                name: "changed".into(),
            }],
        )
        .unwrap();
    editor.undo().unwrap();
    assert!(Arc::ptr_eq(editor.project(), &original));
    studio.timeline_pointer(None, None, false, false, [false; 3]);
    assert!(Arc::ptr_eq(studio.project().unwrap(), &original));
    assert!(
        studio
            .view
            .error
            .as_deref()
            .unwrap()
            .contains("changed during")
    );
}

#[test]
fn modifier_selection_and_duplicate_remain_one_reversible_group() {
    let mut studio = studio();
    studio.select_group(Id(2), false, false).unwrap();
    studio.select_group(Id(3), false, true).unwrap();
    assert_eq!(studio.view.workspace.timeline.selected.len(), 2);
    studio.timeline_action(TimelineAction::Duplicate).unwrap();
    assert_eq!(studio.project().unwrap().clips.len(), 4);
    assert_eq!(studio.view.workspace.timeline.selected.len(), 2);
    studio.activate(Action::Undo);
    assert_eq!(studio.project().unwrap().clips.len(), 2);
    studio.select_group(Id(2), false, false).unwrap();
    studio.select_group(Id(3), true, false).unwrap();
    assert_eq!(studio.view.workspace.timeline.selected.len(), 2);
}
