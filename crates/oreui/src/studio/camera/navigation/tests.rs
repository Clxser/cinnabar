use super::*;
use recording_studio::{
    Id, MediaTime, Project,
    camera::{CameraPose, SavedCameraView},
    model::{MediaKind, MediaSource},
    preview::CameraPreview,
};
use std::sync::Arc;

/// The saved-view panel uses only admitted metadata and the native fixture font.
fn menu(changed: bool) -> MenuView {
    let mut project = Project::new("Named views");
    let mut source = MediaSource {
        id: Id(1),
        name: "Take".into(),
        path: "take.cinrec".into(),
        kind: MediaKind::Replay,
        duration: MediaTime(2_000_000),
        size: [1280, 720],
        frame_rate: None,
        sha256: Some([7; 32]),
    };
    for index in 0..9 {
        project.camera_views.push(
            SavedCameraView::new(
                Id(index + 2),
                format!("View {index}"),
                &source,
                CameraPose::default(),
            )
            .unwrap(),
        );
    }
    if changed {
        source.sha256 = Some([8; 32]);
    }
    project.sources.push(source);
    let mut view = MenuView::new(true, "Player".into());
    let studio = Arc::make_mut(&mut view.studio);
    studio.project = Some(Arc::new(project));
    studio.preview.ready = true;
    studio.preview.camera = Some(CameraPreview {
        world_labels: Default::default(),
        epoch: [1, 0, 0],
        source: Id(1),
        source_time: MediaTime::ZERO,
        pose: CameraPose::default(),
        render_pose: CameraPose::default(),
        interactive: true,
        viewport: [0.0, 0.0, 1.0, 1.0],
        local_actor: 0,
        targets: Arc::from([]),
    });
    studio.camera_workspace.saved_views.selected = Some(Id(2));
    view
}

#[test]
fn changed_sources_offer_explicit_rebinding_and_saved_view_rows_stay_bounded() {
    for changed in [false, true] {
        let view = menu(changed);
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
        let actions: Vec<_> = canvas
            .hits
            .iter()
            .filter_map(|(action, _)| match action {
                MenuAction::Studio(Action::CameraViews(action)) => Some(*action),
                _ => None,
            })
            .collect();
        assert_eq!(
            actions
                .iter()
                .filter(|action| matches!(action, V::Select(_)))
                .count(),
            launcher::menu::studio::SAVED_CAMERA_VIEWS_PER_PAGE
        );
        assert_eq!(actions.contains(&V::Recall(Id(2))), !changed);
        assert_eq!(actions.contains(&V::Rebind(Id(2))), changed);
        assert!(actions.contains(&V::Rename(Id(2))) && actions.contains(&V::Delete(Id(2))));
    }
}

#[test]
fn saved_views_remain_manageable_before_a_native_preview_is_available() {
    let mut view = menu(false);
    let studio = Arc::make_mut(&mut view.studio);
    studio.preview = Default::default();
    studio.camera_workspace.tools = launcher::menu::studio::CameraTools::Views;
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
    let actions: Vec<_> = canvas
        .hits
        .iter()
        .filter_map(|(action, _)| match action {
            MenuAction::Studio(Action::CameraViews(action)) => Some(*action),
            _ => None,
        })
        .collect();
    assert!(actions.contains(&V::Rename(Id(2))) && actions.contains(&V::Delete(Id(2))));
    assert!(!actions.contains(&V::New) && !actions.contains(&V::Recall(Id(2))));
}
