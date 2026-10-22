use super::*;
use crate::review_tests::paint;
use recording_studio::{
    Id, MediaTime, Project, Span,
    model::{TitleClip, TitleStyle},
};
use std::{collections::HashMap, sync::Arc};

/// A short title supplies admitted exportable content without recordings or external media.
fn view() -> MenuView {
    let mut view = MenuView::new(true, "Fixture".into());
    let mut project = Project::new("Recovery controls");
    project.titles.push(TitleClip {
        id: Id(1),
        span: Span::new(MediaTime::ZERO, MediaTime(1_000_000)).unwrap(),
        text: "Title".into(),
        anchor: [0.5, 0.5],
        size: 0.1,
        color: [1.0; 4],
        style: TitleStyle::Body,
        shadow: false,
        fade_in: MediaTime::ZERO,
        fade_out: MediaTime::ZERO,
    });
    project.validate().unwrap();
    Arc::make_mut(&mut view.studio).project = Some(Arc::new(project));
    view
}

#[test]
fn recovery_screen_does_not_publish_blocked_document_header_hits() {
    let mut view = view();
    Arc::make_mut(&mut view.studio).workflow.recovery.visible = true;
    let (_, hits, _) = paint(HashMap::new(), |canvas| {
        draw_workspace(canvas, &view, [1280.0, 720.0]).unwrap()
    });
    assert!(
        hits.iter()
            .any(|(action, _)| *action == MenuAction::Studio(Action::HideRecovery))
    );
    assert!(!hits.iter().any(|(action, _)| matches!(
        action,
        MenuAction::Studio(Action::Save | Action::Export | Action::Library)
    )));
}

#[test]
fn ordered_discard_does_not_publish_cancel_or_duplicate_confirmation_hits() {
    let mut view = view();
    let studio = Arc::make_mut(&mut view.studio);
    studio.discard = Some(launcher::menu::studio::DiscardIntent::NewProject);
    studio.workflow.recovery.busy = true;
    let (_, hits, _) = paint(HashMap::new(), |canvas| {
        discard(canvas, &view, [1280.0, 720.0]).unwrap()
    });
    assert!(!hits.iter().any(|(action, _)| matches!(
        action,
        MenuAction::Studio(Action::CancelDiscard | Action::ConfirmDiscard)
    )));
}

#[test]
fn a_tall_status_in_a_short_window_keeps_document_controls_without_invalid_body_bounds() {
    let mut view = view();
    Arc::make_mut(&mut view.studio).notice =
        Some("Recovery details require attention. ".repeat(80));
    let (_, hits, _) = paint(HashMap::new(), |canvas| {
        draw_workspace(canvas, &view, [900.0, 220.0]).unwrap()
    });
    assert!(
        hits.iter()
            .any(|(action, _)| *action == MenuAction::Studio(Action::Save))
    );
}

#[test]
fn editor_transport_keeps_seek_speed_and_loop_controls_reachable_across_window_sizes() {
    use launcher::menu::studio::{Density, TimelineSize};
    for size in [[1280.0, 720.0], [900.0, 600.0], [640.0, 480.0]] {
        for density in [Density::Compact, Density::Comfortable] {
            for timeline in [
                TimelineSize::Small,
                TimelineSize::Medium,
                TimelineSize::Large,
            ] {
                let mut view = view();
                let studio = Arc::make_mut(&mut view.studio);
                studio.show_library = false;
                studio.workspace.density = density;
                studio.workspace.timeline_size = timeline;
                studio.workspace.monitoring.seek_draft = Some("0.5".into());
                assert_transport_reachable(&view, size);
            }
        }
    }
}

/// Exercises production painting, complete pointer targets and independently usable transport controls.
fn assert_transport_reachable(view: &MenuView, size: [f32; 2]) {
    use launcher::menu::studio::monitoring::MonitoringAction as Monitor;
    use recording_studio::playback::monitoring::JUMP_SECONDS;
    let mut targets = Vec::new();
    let (_, hits, _) = paint(HashMap::new(), |canvas| {
        canvas.capture_focus = true;
        draw(canvas, view, size).unwrap();
        targets = canvas.focus_hits.clone();
    });
    let mut controls: Vec<(Action, ui::UiRect)> = Vec::new();
    for action in [
        Action::Monitoring(Monitor::Start),
        Action::Monitoring(Monitor::Jump(-JUMP_SECONDS)),
        Action::Step(-1),
        Action::TogglePlayback,
        Action::Step(1),
        Action::Monitoring(Monitor::Jump(JUMP_SECONDS)),
        Action::Monitoring(Monitor::End),
        Action::Monitoring(Monitor::StepSpeed(-1)),
        Action::Monitoring(Monitor::Speed(Default::default())),
        Action::Monitoring(Monitor::StepSpeed(1)),
        Action::Monitoring(Monitor::ToggleLoop),
        Action::Monitoring(Monitor::EditSeek),
        Action::Monitoring(Monitor::ApplySeek),
    ] {
        let bounds = hits
            .iter()
            .find_map(|(hit, bounds)| (*hit == MenuAction::Studio(action)).then_some(*bounds))
            .unwrap_or_else(|| {
                panic!(
                    "missing transport control {action:?} at {size:?}, {:?}",
                    view.studio.workspace
                )
            });
        let full = targets
            .iter()
            .find(|(hit, _)| *hit == MenuAction::Studio(action))
            .unwrap()
            .1;
        assert_eq!(bounds, full, "transport control is clipped: {action:?}");
        assert!(bounds.min().x() >= 0.0 && bounds.min().y() >= 0.0);
        assert!(bounds.max().x() <= size[0] && bounds.max().y() <= size[1]);
        for (other, previous) in &controls {
            assert!(
                bounds.max().x() <= previous.min().x()
                    || previous.max().x() <= bounds.min().x()
                    || bounds.max().y() <= previous.min().y()
                    || previous.max().y() <= bounds.min().y(),
                "transport controls overlap: {action:?} and {other:?} at {size:?}"
            );
        }
        controls.push((action, bounds));
    }
}
