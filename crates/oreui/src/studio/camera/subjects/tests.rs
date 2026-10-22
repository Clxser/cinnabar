use super::*;
use launcher::menu::studio::{CAMERA_SUBJECT_ROWS, CameraSubjectResults};
use recording_studio::{
    Id, MediaTime,
    camera::{ActorPose, CameraPose, CameraSubjectSearch, CameraTarget, CameraTargetKind},
    preview::CameraPreview,
};
use std::sync::Arc;

/// A search page is entirely derived from an immutable recorded frame and the native font fixture.
fn menu() -> MenuView {
    let mut view = MenuView::new(true, "Player".into());
    let studio = Arc::make_mut(&mut view.studio);
    studio.preview.ready = true;
    studio.preview.camera = Some(CameraPreview {
        world_labels: Default::default(),
        epoch: [1, 2, 3],
        source: Id(9),
        source_time: MediaTime::ZERO,
        pose: CameraPose::default(),
        render_pose: CameraPose::default(),
        interactive: true,
        viewport: [0.0, 0.0, 1.0, 1.0],
        local_actor: 0,
        targets: (0..10)
            .map(|id| CameraTarget {
                runtime_id: id,
                unique_id: id as i64,
                spawn_revision: 5,
                name: Arc::from("Alpha"),
                kind: if id < 8 {
                    CameraTargetKind::Player
                } else {
                    CameraTargetKind::Entity
                },
                entity_type: None,
                view: Default::default(),
                pose: ActorPose {
                    position: [0.0, 64.0, 4.0],
                    velocity: [0.0; 3],
                    yaw: 0.0,
                    pitch: 0.0,
                    roll: 0.0,
                    eye_offset: None,
                },
                teleported: false,
            })
            .collect(),
    });
    studio.camera_workspace.subject_search = CameraSubjectSearch::new("ALP").unwrap();
    studio.camera_workspace.subject_first = CAMERA_SUBJECT_ROWS;
    studio.camera_workspace.subject_results = Some(CameraSubjectResults::new(
        studio.preview.camera.as_ref().unwrap(),
        &studio.camera_workspace.subject_search,
        studio.camera_workspace.subject_filter,
    ));
    view
}

#[test]
fn native_search_pages_are_bounded_and_bind_actions_to_the_captured_frame_and_player_spawn() {
    let view = menu();
    let (mut nodes, mut next, mut layouts) =
        (Vec::new(), 1, ui::TextLayoutCache::new(128, 1024 * 1024));
    let font = ui_engine::test_support::fixture_font();
    let metrics = ui_engine::text_metrics::TextMetrics::for_viewport(
        [1280, 720],
        ui::DpiScale::new(1.0).unwrap(),
        Some(2),
    );
    let mut canvas = Canvas::new(&mut nodes, &mut next, &mut layouts, &font, metrics, 0, None);
    draw(&mut canvas, &view, [0.0, 0.0, 500.0, 1200.0]).unwrap();
    let selected: Vec<_> = canvas
        .hits
        .iter()
        .filter_map(|(action, _)| match action {
            MenuAction::Studio(Action::CameraSubjects(S::Select {
                source,
                epoch,
                target,
            })) => Some((*source, *epoch, *target)),
            _ => None,
        })
        .collect();
    assert_eq!(selected.len(), CAMERA_SUBJECT_ROWS);
    let preview = view.studio.preview.camera.as_ref().unwrap();
    for ((source, epoch, reference), target) in selected
        .into_iter()
        .zip(&preview.targets[CAMERA_SUBJECT_ROWS..CAMERA_SUBJECT_ROWS * 2])
    {
        assert_eq!(source, preview.source);
        assert_eq!(epoch, preview.epoch);
        assert_eq!(reference, target.reference());
        assert_eq!(target.kind, CameraTargetKind::Player);
    }
}

#[test]
fn shots_without_a_preview_never_expose_hidden_saved_view_controls() {
    let mut view = MenuView::new(true, "Player".into());
    let studio = Arc::make_mut(&mut view.studio);
    studio.project = Some(Arc::new(recording_studio::Project::new("No frame")));
    studio.camera_workspace.tools = launcher::menu::studio::CameraTools::Shots;
    let (mut nodes, mut next, mut layouts) =
        (Vec::new(), 1, ui::TextLayoutCache::new(128, 1024 * 1024));
    let font = ui_engine::test_support::fixture_font();
    let metrics = ui_engine::text_metrics::TextMetrics::for_viewport(
        [1280, 720],
        ui::DpiScale::new(1.0).unwrap(),
        Some(2),
    );
    let mut canvas = Canvas::new(&mut nodes, &mut next, &mut layouts, &font, metrics, 0, None);
    super::super::creation(&mut canvas, &view, [0.0, 0.0, 500.0, 1200.0]).unwrap();
    assert!(
        !canvas
            .hits
            .iter()
            .any(|(action, _)| matches!(action, MenuAction::Studio(Action::CameraViews(_))))
    );
}

#[test]
fn subject_search_clear_label_fits_its_control_at_small_and_large_gui_scales() {
    let view = menu();
    let font = ui_engine::test_support::fixture_font();
    for size in [[640, 480], [900, 600], [1280, 720]] {
        for gui_scale in [1, 2, 3] {
            let (mut nodes, mut next, mut layouts) =
                (Vec::new(), 1, ui::TextLayoutCache::new(128, 1024 * 1024));
            let metrics = ui_engine::text_metrics::TextMetrics::for_viewport(
                size,
                ui::DpiScale::new(1.0).unwrap(),
                Some(gui_scale),
            );
            let mut canvas =
                Canvas::new(&mut nodes, &mut next, &mut layouts, &font, metrics, 0, None);
            let width = canvas.r(26.0);
            draw(&mut canvas, &view, [0.0, 0.0, width, 1200.0]).unwrap();
            let clear = canvas
                .hits
                .iter()
                .find_map(|(action, bounds)| {
                    (*action == MenuAction::Studio(Action::CameraSubjects(S::Clear)))
                        .then_some(*bounds)
                })
                .unwrap();
            let label_width = canvas
                .measure("Clear", crate::theme::SECONDARY_BUTTON)
                .unwrap();
            assert!(clear.width() >= label_width + canvas.r(1.0));
            assert!(clear.max().x() <= width);
        }
    }
}
