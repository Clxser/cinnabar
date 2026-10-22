use super::*;
use launcher::menu::studio::graph::{GraphPublication, GraphWorkspace};
use recording_studio::{
    Id, MediaTime, Project, Span,
    camera::{CameraChannel, CameraPose, CameraRigKind, CameraShot},
    graph::{GraphChannel, GraphCurve, GraphKey},
};
use std::sync::Arc;

#[test]
fn graph_key_fields_and_plot_remain_reachable_at_supported_workspace_widths() {
    for width in [520.0, 780.0, 1200.0] {
        let mut view = MenuView::new(true, "Fixture".into());
        let mut project = Project::new("Camera curve");
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
        let project = Arc::new(project);
        let channel = GraphChannel::Camera(CameraChannel::X);
        let curve = Arc::new(
            GraphCurve::new_reusing(project.clone(), Id(1), channel, CameraPose::default(), None)
                .unwrap(),
        );
        let studio = Arc::make_mut(&mut view.studio);
        studio.selected = Some(Id(1));
        studio.project = Some(project.clone());
        studio.camera_workspace.graph = GraphWorkspace {
            open: true,
            channel: Some(channel),
            selected: Some(GraphKey {
                item: Id(1),
                channel,
                at: MediaTime::ZERO,
            }),
            publication: Some(GraphPublication { project, curve }),
            ..Default::default()
        };
        let (mut nodes, mut next, mut layouts) =
            (Vec::new(), 1, ui::TextLayoutCache::new(128, 1024 * 1024));
        let font = ui_engine::test_support::fixture_font();
        let metrics = ui_engine::text_metrics::TextMetrics::for_viewport(
            [1280, 720],
            ui::DpiScale::new(1.0).unwrap(),
            Some(2),
        );
        let mut canvas = Canvas::new(&mut nodes, &mut next, &mut layouts, &font, metrics, 0, None);
        draw(&mut canvas, &view, [0.0, 0.0, width, 280.0]).unwrap();
        let actions = canvas
            .hits
            .iter()
            .filter_map(|(action, bounds)| {
                if let MenuAction::Studio(Action::Graph(action)) = action {
                    assert!(
                        bounds.min().x() >= 0.0
                            && bounds.max().x() <= width
                            && bounds.min().y() >= 0.0
                            && bounds.max().y() <= 280.0
                    );
                    Some(*action)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for required in [
            GraphAction::Toggle,
            GraphAction::AddKey,
            GraphAction::DeleteKey,
            GraphAction::Reset,
            GraphAction::Edit(GraphField::Time),
            GraphAction::Edit(GraphField::Value),
        ] {
            assert!(
                actions.contains(&required),
                "missing{required:?} at width{width}"
            );
        }
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, GraphAction::Scrub { .. }))
        );
    }
}
