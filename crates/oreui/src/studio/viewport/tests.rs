use super::*;
use recording_studio::{
    Id,
    camera::{ActorPose, CameraPose, CameraTarget, CameraTargetKind},
    preview::CameraPreview,
};

#[test]
fn reset_remains_enabled_while_the_inspection_preview_is_loading() {
    let actions = view_actions(false);
    assert!(
        actions
            .iter()
            .any(|(_, action, enabled)| *action == Action::ResetCameraView && *enabled)
    );
    assert!(
        actions
            .iter()
            .any(|(_, action, enabled)| *action == Action::ApplyCameraView && !*enabled)
    );
}

#[test]
fn global_geometry_window_is_independent_from_timestamp_edit_markers_and_clips_trim_endpoints() {
    use recording_studio::{
        Span,
        camera::{CameraKey, CameraMode, CameraRigKind, CompiledCameraPath, PathTiming},
        effect::Easing,
    };
    let span = Span::new(MediaTime::ZERO, MediaTime(300_000_000)).unwrap();
    let mut shot = CameraShot::from_preview(
        Id(1),
        span,
        CameraRigKind::Path,
        CameraPose::default(),
        None,
    )
    .unwrap();
    shot.mode = CameraMode::Path {
        keys: (0..=300)
            .map(|index| CameraKey {
                at: MediaTime(index * 1_000_000),
                pose: CameraPose {
                    position: [
                        if index < 300 {
                            index as f64 * 0.001
                        } else {
                            1000.0
                        },
                        0.0,
                        0.0,
                    ],
                    ..Default::default()
                },
                easing: Easing::Linear,
                handles: None,
            })
            .collect(),
        smooth_position: false,
    };
    let shot = shot
        .with_path_timing(PathTiming::GlobalDistance)
        .unwrap()
        .trimmed(Span::new(MediaTime(20_000_000), MediaTime(40_000_000)).unwrap())
        .unwrap();
    let cache = Arc::new(CompiledCameraPath::new(&shot).unwrap());
    let mut view = MenuView::new(true, "Player".into());
    let studio = Arc::make_mut(&mut view.studio);
    studio.position = MediaTime(30_000_000);
    studio.camera_workspace.path_cache = Some(launcher::menu::studio::CameraPathCache {
        shot: shot.id,
        epoch: [1, 0],
        draft: None,
        path: Arc::clone(&cache),
    });
    assert!(!key_window(&view, &shot).contains(&299));
    let window = geometry_window(&view, &shot);
    assert_eq!(window, 299..300);
    assert!(window.len() <= MAX_VISIBLE_KEYS);
    let points = cache
        .geometry_samples::<SEGMENT_POINTS>(window.start)
        .unwrap();
    for (point, at) in [
        (points[0], shot.span.start),
        (points[SEGMENT_POINTS - 1], shot.span.end),
    ] {
        let expected = cache.sample(at).unwrap().position;
        assert!(
            point
                .into_iter()
                .zip(expected)
                .all(|(actual, expected)| (actual - expected).abs() < 1e-9)
        );
    }
}

/// Identical display names cannot turn a floating-text entity into a player selection marker.
fn target(id: u64, kind: CameraTargetKind) -> CameraTarget {
    CameraTarget {
        runtime_id: id,
        unique_id: id as i64,
        spawn_revision: 1,
        name: "Recorded label".into(),
        kind,
        entity_type: None,
        view: Default::default(),
        pose: ActorPose {
            position: [0.0, 64.0, 8.0],
            velocity: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            eye_offset: None,
        },
        teleported: false,
    }
}

#[test]
fn subject_markers_require_the_player_tool_and_stay_small_at_large_gui_scales() {
    for gui in [2, 4] {
        let mut view = MenuView::new(true, "Player".into());
        Arc::make_mut(&mut view.studio).preview.camera = Some(CameraPreview {
            world_labels: Default::default(),
            epoch: [1, 0, 0],
            source: Id(1),
            source_time: MediaTime::ZERO,
            pose: CameraPose::default(),
            render_pose: CameraPose::default(),
            interactive: true,
            viewport: [0.0, 0.0, 1.0, 1.0],
            local_actor: 0,
            targets: Arc::from([
                target(1, CameraTargetKind::Entity),
                target(2, CameraTargetKind::Player),
            ]),
        });
        let (mut nodes, mut next, mut layouts) =
            (Vec::new(), 1, ui::TextLayoutCache::new(128, 1024 * 1024));
        let font = ui_engine::test_support::fixture_font();
        let metrics = ui_engine::text_metrics::TextMetrics::for_viewport(
            [1280, 720],
            ui::DpiScale::new(1.0).unwrap(),
            Some(gui),
        );
        let mut canvas = Canvas::new(&mut nodes, &mut next, &mut layouts, &font, metrics, 0, None);
        let bounds = [0.0, 0.0, 800.0, 450.0];
        draw(&mut canvas, &view, bounds, bounds).unwrap();
        assert!(
            !canvas
                .hits
                .iter()
                .any(|(action, _)| matches!(action, MenuAction::Studio(Action::CameraSubject(_))))
        );
        canvas.hits.clear();
        Arc::make_mut(&mut view.studio).workspace.viewport_tool = ViewportTool::Players;
        draw(&mut canvas, &view, bounds, bounds).unwrap();
        let subjects: Vec<_> = canvas
            .hits
            .iter()
            .filter_map(|(action, rect)| match action {
                MenuAction::Studio(Action::CameraSubject(target)) => {
                    Some((target.runtime_id, rect))
                }
                _ => None,
            })
            .collect();
        assert_eq!(subjects.len(), 1);
        assert_eq!(subjects[0].0, 2);
        assert_eq!(subjects[0].1.width(), 18.0);
        assert_eq!(subjects[0].1.height(), 18.0);
    }
}
