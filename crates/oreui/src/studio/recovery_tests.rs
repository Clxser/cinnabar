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
