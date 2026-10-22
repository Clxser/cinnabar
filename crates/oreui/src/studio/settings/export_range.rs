//! Project timeline export bounds have exact typed fields and explicit playhead actions.

use super::*;
use launcher::menu::{MenuField, studio::ExportRangeField};
use recording_studio::{MediaTime, Span, export::range::FrameRange};

/// Range controls share the selected output cadence and never alter clip Source In/Out.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    let Some(project) = &view.studio.project else {
        return Ok(y);
    };
    let duration = project.duration();
    canvas.text_line(
        "Export range — project timeline",
        [left, y],
        right - left,
        BODY,
        TEXT,
    )?;
    y += canvas.r(3.5);
    let admitted = FrameRange::new(project.export.fps, duration, project.export.range);
    let status = match &admitted {
        Ok(range) => format!(
            "{} · {} output frames",
            if project.export.range.is_none() {
                "Full project"
            } else {
                "Custom range"
            },
            range.frames()
        ),
        Err(error) => error.to_string(),
    };
    y += canvas.text(
        &status,
        [left, y],
        right - left,
        CAPTION,
        TEXT_DIMMER,
        false,
    )? + canvas.r(1.0);
    let enabled = !view.studio.loading && !view.studio.export.phase.busy();
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(3.8)],
        "Full project",
        Action::FullExportRange,
        enabled,
        Variant::Secondary,
    )?;
    y += canvas.r(4.8);
    if let Ok(full) = Span::new(MediaTime::ZERO, duration) {
        let span = project.export.range.unwrap_or(full);
        for &field in ExportRangeField::ALL {
            canvas.text_line(field.label(), [left, y], right - left, CAPTION, TEXT_DIMMER)?;
            y += canvas.r(2.7);
            let value = format!("{:.6}", field.value(span).seconds());
            let draft = view
                .studio
                .workflow
                .export_range_draft
                .as_ref()
                .filter(|(kind, _)| *kind == field)
                .map(|(_, text)| text.as_ref());
            let gap = canvas.r(0.8);
            let button = canvas.r(19.0).min((right - left) * 0.43);
            let edge = right - button;
            widgets::text_field(
                canvas,
                view,
                [left, y, edge - gap, y + canvas.r(3.8)],
                draft.unwrap_or(&value),
                field.label(),
                view.field == Some(MenuField::StudioExportRange(field)),
                enabled.then_some(MenuAction::Studio(Action::EditExportRange(field))),
            )?;
            control(
                canvas,
                view,
                [edge, y, right, y + canvas.r(3.8)],
                field.playhead_label(),
                Action::SetExportRangeAtPlayhead(field),
                enabled,
                Variant::Neutral,
            )?;
            y += canvas.r(4.8);
            if draft.is_some() {
                let middle = (left + right) * 0.5;
                control(
                    canvas,
                    view,
                    [left, y, middle - gap * 0.5, y + canvas.r(3.8)],
                    "Apply",
                    Action::ApplyExportRange,
                    enabled,
                    Variant::Primary,
                )?;
                control(
                    canvas,
                    view,
                    [middle + gap * 0.5, y, right, y + canvas.r(3.8)],
                    "Cancel",
                    Action::CancelExportRange,
                    true,
                    Variant::Secondary,
                )?;
                y += canvas.r(4.8);
            }
        }
    }
    y += canvas.text("Project timeline seconds. In is inclusive; Out is exclusive. Values snap to output FPS. These bounds affect export only.", [left,y], right-left, CAPTION, TEXT_DIMMER, false)? + canvas.r(2.2);
    Ok(y)
}
