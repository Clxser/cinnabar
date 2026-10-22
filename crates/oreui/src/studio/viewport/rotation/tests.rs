use super::*;
use recording_studio::{
    Id, Project, Span,
    camera::{CameraPose, CameraRigKind},
    preview::CameraPreview,
};
use std::sync::Arc;
use ui::{UiNode, UiVisual};

/// A selected anonymous camera has no files or account details and shares the current native viewpoint.
fn view(path: bool) -> MenuView {
    let mut view = MenuView::new(true, "Fixture".into());
    let mut project = Project::new("Camera rotation");
    project.shots.push(
        CameraShot::from_preview(
            Id(1),
            Span::new(MediaTime::ZERO, MediaTime(2_000_000)).unwrap(),
            if path {
                CameraRigKind::Path
            } else {
                CameraRigKind::Free
            },
            CameraPose::default(),
            None,
        )
        .unwrap(),
    );
    let studio = Arc::make_mut(&mut view.studio);
    studio.selected = Some(Id(1));
    studio.project = Some(Arc::new(project));
    studio.workspace.viewport_tool = ViewportTool::Rotate;
    studio.preview.ready = true;
    studio.preview.camera = Some(CameraPreview {
        world_labels: Default::default(),
        epoch: [1, 0, 0],
        source: Id(7),
        source_time: MediaTime::ZERO,
        pose: CameraPose::default(),
        render_pose: CameraPose::default(),
        interactive: true,
        viewport: [0.0, 0.0, 1.0, 1.0],
        local_actor: 0,
        targets: Arc::from([]),
    });
    view
}

/// Collects semantic hit targets and the actual mesh used to paint hover/active feedback.
fn paint(view: &MenuView, bounds: Bounds) -> (Vec<(MenuAction, ui::UiRect)>, Vec<UiNode>) {
    let (mut nodes, mut next, mut layouts) =
        (Vec::new(), 1, ui::TextLayoutCache::new(128, 1024 * 1024));
    let font = ui_engine::test_support::fixture_font();
    let metrics = ui_engine::text_metrics::TextMetrics::for_viewport(
        [1280, 720],
        ui::DpiScale::new(1.0).unwrap(),
        Some(2),
    );
    let mut canvas = Canvas::new(&mut nodes, &mut next, &mut layouts, &font, metrics, 0, None);
    super::super::draw(&mut canvas, view, bounds, bounds).unwrap();
    let hits = std::mem::take(&mut canvas.hits);
    drop(canvas);
    (hits, nodes)
}

/// The ring's first quad is yaw; width is reconstructed in image pixels, independent of tree structure.
fn yaw_stroke(nodes: &[UiNode], bounds: Bounds) -> (f32, [u8; 4]) {
    let mesh = nodes
        .iter()
        .find_map(|node| match node.visual() {
            UiVisual::Mesh(mesh) => Some(mesh),
            _ => None,
        })
        .unwrap();
    let vertices = mesh.vertices();
    let distance = [
        (vertices[0].position[0] - vertices[1].position[0]) * (bounds[2] - bounds[0]),
        (vertices[0].position[1] - vertices[1].position[1]) * (bounds[3] - bounds[1]),
    ];
    (distance[0].hypot(distance[1]), vertices[0].color)
}

#[test]
fn free_at_own_eye_and_exact_path_keys_have_connected_rings_and_numeric_values() {
    for path in [false, true] {
        let view = view(path);
        let bounds = [50.0, 30.0, 850.0, 480.0];
        let (hits, _) = paint(&view, bounds);
        for axis in CameraRotationAxis::ALL {
            let ring_hits: Vec<_> = hits
                .iter()
                .filter_map(|(action, hit)| match action {
                    MenuAction::Studio(Action::CameraRotationControl {
                        shot,
                        at,
                        axis: actual,
                        bounds: geometry,
                    }) if *actual == axis => Some((*shot, *at, *geometry, hit)),
                    _ => None,
                })
                .collect();
            assert!(!ring_hits.is_empty());
            for (shot, at, geometry, hit) in ring_hits {
                assert_eq!(shot, Id(1));
                assert_eq!(at, MediaTime::ZERO);
                assert_eq!(geometry, ring(bounds, axis).unwrap());
                assert!(hit.width() > 0.0 && hit.height() > 0.0);
            }
            assert!(hits.iter().any(|(action, _)| *action
                == MenuAction::Studio(Action::EditCameraValue(axis.parameter()))));
        }
    }
}

#[test]
fn hover_and_active_state_change_visible_ring_color_and_stroke_width() {
    let mut view = view(false);
    let bounds = [0.0, 0.0, 800.0, 450.0];
    let (_, idle) = paint(&view, bounds);
    let geometry = ring(bounds, CameraRotationAxis::Yaw).unwrap();
    view.hovered = Some(MenuAction::Studio(Action::CameraRotationControl {
        shot: Id(1),
        at: MediaTime::ZERO,
        axis: CameraRotationAxis::Yaw,
        bounds: geometry,
    }));
    let (_, hovered) = paint(&view, bounds);
    Arc::make_mut(&mut view.studio).camera_workspace.rotation =
        Some(launcher::menu::studio::CameraRotationHandle {
            shot: Id(1),
            at: MediaTime::ZERO,
            axis: CameraRotationAxis::Yaw,
            bounds: geometry,
        });
    let (_, active) = paint(&view, bounds);
    let (idle_width, idle_color) = yaw_stroke(&idle, bounds);
    let (hover_width, hover_color) = yaw_stroke(&hovered, bounds);
    let (active_width, active_color) = yaw_stroke(&active, bounds);
    assert!(hover_width > idle_width + 0.5);
    assert!(active_width > hover_width + 0.3);
    assert_ne!(idle_color, hover_color);
    assert_eq!(hover_color, active_color);
}

#[test]
fn captured_ring_keeps_geometry_after_viewport_change_and_clips_hits_after_small_resize() {
    let mut view = view(false);
    let original = ring([50.0, 30.0, 850.0, 480.0], CameraRotationAxis::Yaw).unwrap();
    Arc::make_mut(&mut view.studio).camera_workspace.rotation =
        Some(launcher::menu::studio::CameraRotationHandle {
            shot: Id(1),
            at: MediaTime::ZERO,
            axis: CameraRotationAxis::Yaw,
            bounds: original,
        });
    for bounds in [[100.0, 80.0, 900.0, 530.0], [0.0, 0.0, 100.0, 60.0]] {
        let (hits, _) = paint(&view, bounds);
        for (action, hit) in hits {
            if let MenuAction::Studio(Action::CameraRotationControl {
                axis: CameraRotationAxis::Yaw,
                bounds: frozen,
                ..
            }) = action
            {
                assert_eq!(frozen, original);
            }
            assert!(hit.width() > 0.0 && hit.height() > 0.0);
        }
    }
}

#[test]
fn rings_require_the_rotate_tool_an_exact_key_ready_native_image_and_unlocked_track() {
    for excluded in 0..6 {
        let mut view = view(true);
        let studio = Arc::make_mut(&mut view.studio);
        let mut bounds = [0.0, 0.0, 800.0, 450.0];
        match excluded {
            0 => studio.workspace.viewport_tool = ViewportTool::Navigate,
            1 => studio.position = MediaTime(500_000),
            2 => studio.preview.ready = false,
            3 => studio.preview.camera.as_mut().unwrap().interactive = false,
            4 => {
                studio
                    .workspace
                    .timeline
                    .locked
                    .insert(recording_studio::tracks::Track::Camera);
            }
            _ => bounds = [0.0, 0.0, 160.0, 100.0],
        }
        let (hits, _) = paint(&view, bounds);
        assert!(!hits.iter().any(|(action, _)| matches!(
            action,
            MenuAction::Studio(Action::CameraRotationControl { .. })
        )));
    }
}
