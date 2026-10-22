use super::*;
use launcher::menu::studio::{CameraTools, CameraViewsAction as V};
use recording_studio::{
    Id, MediaTime, Project,
    camera::{CameraPose, SavedCameraView},
    model::{MediaKind, MediaSource},
};
use std::sync::Arc;

#[test]
fn saved_views_are_reachable_in_their_section_without_scrolling_past_shot_creation() {
    let source = MediaSource {
        id: Id(1),
        name: "Take".into(),
        path: "take.cinreplay".into(),
        kind: MediaKind::Replay,
        duration: MediaTime(1_000_000),
        size: [1280, 720],
        frame_rate: None,
        sha256: None,
    };
    let mut project = Project::new("Camera tools");
    project
        .camera_views
        .push(SavedCameraView::new(Id(2), "Wide".into(), &source, CameraPose::default()).unwrap());
    project.sources.push(source);
    for section in CameraTools::ALL {
        let mut view = MenuView::new(true, "Player".into());
        let studio = Arc::make_mut(&mut view.studio);
        studio.project = Some(Arc::new(project.clone()));
        studio.workspace.toolbox = Toolbox::Cameras;
        studio.camera_workspace.tools = *section;
        let (mut nodes, mut next, mut layouts) =
            (Vec::new(), 1, ui::TextLayoutCache::new(128, 1024 * 1024));
        let font = ui_engine::test_support::fixture_font();
        let metrics = ui_engine::text_metrics::TextMetrics::for_viewport(
            [1280, 720],
            ui::DpiScale::new(1.0).unwrap(),
            Some(2),
        );
        let mut canvas = Canvas::new(&mut nodes, &mut next, &mut layouts, &font, metrics, 0, None);
        draw(&mut canvas, &view, [0.0, 0.0, 220.0, 400.0]).unwrap();
        for tab in CameraTools::ALL {
            assert!(
                canvas.hits.iter().any(|(action, _)| *action
                    == MenuAction::Studio(Action::CameraViews(V::Section(*tab))))
            );
        }
        let selected = canvas.hits.iter().find(|(action, _)| {
            *action == MenuAction::Studio(Action::CameraViews(V::Select(Id(2))))
        });
        assert_eq!(selected.is_some(), *section == CameraTools::Views);
        if let Some((_, bounds)) = selected {
            assert!(bounds.min().y() >= 0.0 && bounds.max().y() <= 400.0);
        }
    }
}

#[test]
fn tool_buttons_leave_the_scrollbar_gutter_clear_in_narrow_panels() {
    let mut view = MenuView::new(true, "Player".into());
    let studio = Arc::make_mut(&mut view.studio);
    studio.workspace.toolbox = Toolbox::Effects;
    for width in [110.0, 220.0] {
        let (mut nodes, mut next, mut layouts) =
            (Vec::new(), 1, ui::TextLayoutCache::new(128, 1024 * 1024));
        let font = ui_engine::test_support::fixture_font();
        let metrics = ui_engine::text_metrics::TextMetrics::for_viewport(
            [1280, 720],
            ui::DpiScale::new(1.0).unwrap(),
            Some(2),
        );
        let mut canvas = Canvas::new(&mut nodes, &mut next, &mut layouts, &font, metrics, 0, None);
        draw(&mut canvas, &view, [0.0, 0.0, width, 160.0]).unwrap();
        let track = canvas
            .scrolls
            .iter()
            .find_map(|scroll| scroll.track)
            .unwrap();
        assert!(!canvas.hits.is_empty());
        for (_, bounds) in &canvas.hits {
            assert!(bounds.max().x() < track.min().x());
        }
    }
}
