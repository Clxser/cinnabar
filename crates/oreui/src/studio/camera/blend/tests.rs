use super::*;
use recording_studio::{
    Id, MediaTime, Span,
    camera::{CameraBlend, CameraPose, CameraRigKind},
    effect::Easing,
    tracks::Track,
};
use std::sync::Arc;

#[test]
fn native_entry_controls_remain_reachable_and_disjoint_at_inspector_widths() {
    for width in [150.0, 220.0, 400.0] {
        for blend in [false, true] {
            for locked in [false, true] {
                let mut view = MenuView::new(true, "Player".into());
                if locked {
                    Arc::make_mut(&mut view.studio)
                        .workspace
                        .timeline
                        .locked
                        .insert(Track::Camera);
                }
                let mut shot = CameraShot::from_preview(
                    Id(1),
                    Span::new(MediaTime::ZERO, MediaTime(2_000_000)).unwrap(),
                    CameraRigKind::Free,
                    CameraPose::default(),
                    None,
                )
                .unwrap();
                shot.blend_in = blend.then_some(CameraBlend {
                    duration: CameraBlend::DEFAULT_DURATION,
                    easing: Easing::Smooth,
                });
                let (mut nodes, mut next, mut layouts) =
                    (Vec::new(), 1, ui::TextLayoutCache::new(128, 1024 * 1024));
                let font = ui_engine::test_support::fixture_font();
                let metrics = ui_engine::text_metrics::TextMetrics::for_viewport(
                    [1280, 720],
                    ui::DpiScale::new(1.0).unwrap(),
                    Some(2),
                );
                let mut canvas =
                    Canvas::new(&mut nodes, &mut next, &mut layouts, &font, metrics, 0, None);
                let bottom = draw(&mut canvas, &view, [0.0, 0.0, width, 500.0], &shot).unwrap();
                assert!(bottom.is_finite() && bottom > 0.0 && bottom < 500.0);
                let controls: Vec<_> = canvas
                    .hits
                    .iter()
                    .filter_map(|(action, bounds)| match action {
                        MenuAction::Studio(Action::CameraBlend(action)) => Some((
                            *action,
                            [
                                bounds.min().x,
                                bounds.min().y,
                                bounds.max().x,
                                bounds.max().y,
                            ],
                        )),
                        _ => None,
                    })
                    .collect();
                let actions: Vec<_> = controls.iter().map(|(action, _)| *action).collect();
                assert_eq!(actions.contains(&B::Toggle), !locked);
                for action in [B::Duration(-1), B::Duration(1), B::CycleEasing] {
                    assert_eq!(actions.contains(&action), blend && !locked);
                }
                for (_, [left, top, right, bottom]) in &controls {
                    assert!(*left >= 0.0 && *right <= width && right > left && bottom > top);
                }
                for (index, (_, a)) in controls.iter().enumerate() {
                    for (_, b) in &controls[index + 1..] {
                        assert!(a[2] <= b[0] || b[2] <= a[0] || a[3] <= b[1] || b[3] <= a[1]);
                    }
                }
            }
        }
    }
}
