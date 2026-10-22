use super::*;
use recording_studio::camera::{CameraChannel, CameraRigKind, CameraShot};

/// Offline graph input uses native Studio state without starting storage or preview workers.
fn studio() -> Studio {
    let mut studio = Studio::new(PathBuf::from("unused-graph-fixture"));
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
    studio.editor = Some(Editor::new(project).unwrap());
    let view = Arc::make_mut(&mut studio.view);
    view.show_library = false;
    view.selected = Some(Id(1));
    view.visible_duration = MediaTime(2_000_000);
    studio.graph_action(GraphAction::Toggle).unwrap();
    studio.publish_editor();
    studio
}

/// The graph captures its own plot geometry and original key identity at press time.
fn action() -> Option<launcher::menu::MenuAction> {
    Some(launcher::menu::MenuAction::Studio(Action::Graph(
        GraphAction::Drag {
            key: GraphKey {
                item: Id(1),
                channel: GraphChannel::Camera(CameraChannel::X),
                at: MediaTime::ZERO,
            },
            control: GraphControl::Key,
            bounds: [0, 0, 400, 200],
        },
    )))
}

#[test]
fn graph_drag_previews_without_committing_and_release_is_one_undo_step() {
    let mut studio = studio();
    let original = studio.project().unwrap().clone();
    let revision = studio.editor.as_ref().unwrap().revision();
    assert!(studio.graph_pointer(action(), Some([0.0, 100.0]), true, true));
    assert!(studio.graph_pointer(None, Some([200.0, 50.0]), true, false));
    assert!(Arc::ptr_eq(studio.project().unwrap(), &original));
    assert_eq!(studio.editor.as_ref().unwrap().revision(), revision);
    let draft = studio.graph.project.clone().unwrap();
    assert!(
        draft.shots[0]
            .animation
            .curves
            .contains_key(&CameraChannel::X)
    );
    assert!(studio.graph_pointer(None, None, false, false));
    assert_eq!(studio.project().unwrap().as_ref(), draft.as_ref());
    assert_eq!(studio.editor.as_ref().unwrap().revision(), revision + 1);
    studio.editor.as_mut().unwrap().undo().unwrap();
    assert_eq!(studio.project().unwrap().as_ref(), original.as_ref());
    studio.publish_editor();
    assert!(studio.view.camera_workspace.graph.selected.is_none());
}

#[test]
fn cancellation_or_track_lock_never_publishes_a_camera_curve() {
    let mut studio = studio();
    let original = studio.project().unwrap().clone();
    studio.graph_pointer(action(), Some([0.0, 100.0]), true, true);
    studio.graph_pointer(None, Some([200.0, 50.0]), true, false);
    studio.cancel_pointer();
    assert!(studio.graph.project.is_none());
    assert!(Arc::ptr_eq(studio.project().unwrap(), &original));
    Arc::make_mut(&mut studio.view)
        .workspace
        .timeline
        .locked
        .insert(recording_studio::tracks::Track::Camera);
    studio.graph_pointer(action(), Some([0.0, 100.0]), true, true);
    studio.graph_pointer(None, Some([200.0, 50.0]), true, false);
    studio.graph_pointer(None, None, false, false);
    assert!(Arc::ptr_eq(studio.project().unwrap(), &original));
}

#[test]
fn document_modal_seek_selection_and_locks_cancel_frozen_graph_gestures() {
    for change in 0..5 {
        let mut studio = studio();
        let original = studio.project().unwrap().clone();
        studio.graph_pointer(action(), Some([0.0, 100.0]), true, true);
        studio.graph_pointer(None, Some([200.0, 50.0]), true, false);
        match change {
            0 => Arc::make_mut(&mut studio.view).workflow.show_settings = true,
            1 => studio
                .transport
                .seek(MediaTime(100_000), MediaTime(2_000_000)),
            2 => Arc::make_mut(&mut studio.view).selected = None,
            3 => {
                Arc::make_mut(&mut studio.view)
                    .workspace
                    .timeline
                    .locked
                    .insert(recording_studio::tracks::Track::Camera);
            }
            _ => {
                Arc::make_mut(&mut studio.view)
                    .camera_workspace
                    .graph
                    .channel = Some(GraphChannel::Camera(CameraChannel::Y))
            }
        }
        studio.graph_pointer(None, None, false, false);
        assert!(studio.graph.project.is_none());
        assert!(Arc::ptr_eq(studio.project().unwrap(), &original));
    }
    let mut studio = studio();
    studio.graph_pointer(action(), Some([0.0, 100.0]), true, true);
    studio.graph_pointer(None, Some([200.0, 50.0]), true, false);
    studio.install(Editor::new(Project::new("Replacement")).unwrap(), None);
    studio.publish_editor();
    assert!(studio.graph.project.is_none());
    assert!(!studio.graph_drag_active());
    assert!(studio.view.camera_workspace.graph.publication.is_none());
}

#[test]
fn stale_key_hit_is_rejected_and_moved_key_stays_selected_for_followup_edits() {
    let mut studio = studio();
    let Some(launcher::menu::MenuAction::Studio(Action::Graph(GraphAction::Drag {
        mut key,
        control,
        bounds,
    }))) = action()
    else {
        unreachable!()
    };
    key.channel = GraphChannel::Camera(CameraChannel::Y);
    studio.graph_pointer(
        Some(launcher::menu::MenuAction::Studio(Action::Graph(
            GraphAction::Drag {
                key,
                control,
                bounds,
            },
        ))),
        Some([0.0, 100.0]),
        true,
        true,
    );
    assert!(!studio.graph_drag_active());
    studio.graph_pointer(action(), Some([0.0, 100.0]), true, true);
    assert!(studio.authored_camera_drag_active());
    studio.graph_pointer(None, Some([200.0, 50.0]), true, false);
    studio.graph_pointer(None, None, false, false);
    let selected = studio.view.camera_workspace.graph.selected.unwrap();
    assert!(selected.at > MediaTime::ZERO);
    studio.graph_action(GraphAction::CycleEasing).unwrap();
    assert_eq!(studio.view.camera_workspace.graph.selected, Some(selected));
    studio.graph_action(GraphAction::DeleteKey).unwrap();
    assert!(studio.project().unwrap().shots[0].animation.is_empty());
}

#[test]
fn typed_key_value_is_exact_undoable_and_rejects_stale_or_nonfinite_drafts() {
    use launcher::menu::studio::graph::GraphField;
    let mut studio = studio();
    studio
        .graph_action(GraphAction::Select(GraphKey {
            item: Id(1),
            channel: GraphChannel::Camera(CameraChannel::X),
            at: MediaTime::ZERO,
        }))
        .unwrap();
    let original = studio.project().unwrap().clone();
    studio.begin_graph_value(GraphField::Value).unwrap();
    studio.graph_text.set_text("NaN");
    assert!(studio.apply_graph_value().is_err());
    assert!(Arc::ptr_eq(studio.project().unwrap(), &original));
    studio.graph_text.set_text("3.125");
    studio.apply_graph_value().unwrap();
    assert_eq!(
        studio.project().unwrap().shots[0]
            .sample(MediaTime::ZERO, |_| None)
            .unwrap()
            .position[0],
        3.125
    );
    studio.editor.as_mut().unwrap().undo().unwrap();
    assert_eq!(studio.project().unwrap().as_ref(), original.as_ref());
    studio.publish_editor();
    studio.begin_graph_value(GraphField::Time).unwrap();
    Arc::make_mut(&mut studio.view).workflow.show_settings = true;
    assert!(studio.apply_graph_value().is_err());
}

#[test]
fn graph_plot_reuses_stationary_samples_and_value_framing_stays_finite() {
    assert!((nudge_value(10.0, [-1.0, 1.0], 0.01) - 10.02).abs() < 1e-12);
    let mut studio = studio();
    let points = studio
        .view
        .camera_workspace
        .graph
        .plot
        .as_ref()
        .unwrap()
        .points
        .clone();
    for _ in 0..4 {
        studio.publish_editor();
    }
    assert!(Arc::ptr_eq(
        &points,
        &studio
            .view
            .camera_workspace
            .graph
            .plot
            .as_ref()
            .unwrap()
            .points
    ));
    Arc::make_mut(&mut studio.view).visible_start = MediaTime(10_000);
    studio.publish_editor();
    assert!(!Arc::ptr_eq(
        &points,
        &studio
            .view
            .camera_workspace
            .graph
            .plot
            .as_ref()
            .unwrap()
            .points
    ));
    for range in [[-1e308, 1e308], [0.0, 1e-300], [1e300, 1.1e300]] {
        for factor in [0.25, 1.0, 4.0] {
            let result = transform_range(range, 0.3, factor, 0.125);
            assert!(result[0] < result[1] && result.iter().all(|v| v.is_finite()));
        }
    }
}

#[test]
fn fresh_stale_graph_hits_cannot_seek_behind_overlays_or_after_selection_changes() {
    for change in 0..6 {
        let mut studio = studio();
        studio
            .transport
            .seek(MediaTime(300_000), MediaTime(2_000_000));
        match change {
            0 => Arc::make_mut(&mut studio.view).workflow.recovery.visible = true,
            1 => Arc::make_mut(&mut studio.view).workflow.recovery.busy = true,
            2 => Arc::make_mut(&mut studio.view).workflow.show_settings = true,
            3 => Arc::make_mut(&mut studio.view).show_library = true,
            4 => Arc::make_mut(&mut studio.view).selected = None,
            _ => Arc::make_mut(&mut studio.view).camera_workspace.graph.open = false,
        }
        studio.graph_pointer(action(), Some([0.0, 100.0]), true, true);
        assert_eq!(studio.transport.position(), MediaTime(300_000));
        assert!(!studio.graph_drag_active());
    }
}

#[test]
fn dragging_preserves_the_grab_offset_in_the_unedited_axis() {
    let mut studio = studio();
    studio.graph_pointer(action(), Some([3.0, 105.0]), true, true);
    studio.graph_pointer(None, Some([203.0, 105.0]), true, false);
    studio.graph_pointer(None, None, false, false);
    let key = studio.view.camera_workspace.graph.selected.unwrap();
    let pose = studio.project().unwrap().shots[0]
        .sample(key.at, |_| None)
        .unwrap();
    assert_eq!(pose.position[0], 0.0);
    assert_eq!(key.at, MediaTime(1_000_000));
}

#[test]
fn playback_follow_keeps_the_curve_plot_on_the_revealed_time_window() {
    let mut studio = studio();
    let initial = studio.view.camera_workspace.graph.plot.clone().unwrap();
    Arc::make_mut(&mut studio.view).visible_duration = MediaTime(500_000);
    Arc::make_mut(&mut studio.view).workspace.follow_playhead = true;
    studio
        .transport
        .seek(MediaTime(1_500_000), MediaTime(2_000_000));
    studio.transport.play(MediaTime(2_000_000));
    studio.follow_playhead();
    let plot = studio.view.camera_workspace.graph.plot.as_ref().unwrap();
    assert_eq!(
        plot.window,
        [studio.view.visible_start, studio.view.visible_duration]
    );
    assert!(plot.window[0] > MediaTime::ZERO);
    assert!(plot.points.first().unwrap()[0] >= plot.window[0].0 as f64);
    assert!(!Arc::ptr_eq(&initial.points, &plot.points));
}

#[test]
fn opening_extreme_finite_key_values_never_truncates_the_numeric_draft() {
    use launcher::menu::studio::graph::GraphField;
    let mut studio = studio();
    let curve = studio
        .view
        .camera_workspace
        .graph
        .publication
        .as_ref()
        .unwrap()
        .curve
        .clone();
    let command = graph::set_key_on(
        &curve,
        studio.project().unwrap(),
        Some(MediaTime::ZERO),
        MediaTime::ZERO,
        1e308,
    )
    .unwrap();
    studio.edit("Large fixture", &[command]).unwrap();
    studio.publish_editor();
    studio
        .graph_action(GraphAction::Select(GraphKey {
            item: Id(1),
            channel: curve.channel,
            at: MediaTime::ZERO,
        }))
        .unwrap();
    studio.begin_graph_value(GraphField::Value).unwrap();
    assert_eq!(studio.graph_text.as_str().parse::<f64>().unwrap(), 1e308);
    assert!(studio.graph_text.as_str().len() <= recording_studio::camera::MAX_CAMERA_VALUE_BYTES);
}

#[test]
fn graph_nudges_follow_exact_rational_frames_without_accumulating_rounding() {
    let mut studio = studio();
    let fps = recording_studio::FrameRate {
        numerator: 30_000,
        denominator: 1_001,
    };
    let mut project = studio.project().unwrap().as_ref().clone();
    project.export.fps = fps;
    studio.editor = Some(Editor::new(project).unwrap());
    studio.publish_editor();
    studio
        .graph_action(GraphAction::Select(GraphKey {
            item: Id(1),
            channel: GraphChannel::Camera(CameraChannel::X),
            at: MediaTime::ZERO,
        }))
        .unwrap();
    for index in 1..=10 {
        studio
            .graph_action(GraphAction::Nudge { time: 1, value: 0 })
            .unwrap();
        assert_eq!(
            studio.view.camera_workspace.graph.selected.unwrap().at,
            fps.frame_time(index).unwrap()
        );
    }
    for index in (0..10).rev() {
        studio
            .graph_action(GraphAction::Nudge { time: -1, value: 0 })
            .unwrap();
        assert_eq!(
            studio.view.camera_workspace.graph.selected.unwrap().at,
            fps.frame_time(index).unwrap()
        );
    }
}
