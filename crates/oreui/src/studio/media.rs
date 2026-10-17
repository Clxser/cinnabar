//! Media import and relink controls show real worker progress without blocking the editor.

use super::*;

/// The media panel adds imports at the end of the timeline and relinks the selected source.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), PaintError> {
    let pad = canvas.r(1.2);
    let mut y = area[1];
    let left = area[0];
    let right = area[2];
    canvas.text_line("Media", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(4.0);
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(4.0)],
        if view.studio.workflow.media_busy {
            "Cancel media operation"
        } else {
            "Import media..."
        },
        if view.studio.workflow.media_busy {
            Action::CancelMedia
        } else {
            Action::ImportMedia
        },
        true,
        Variant::Primary,
    )?;
    y += canvas.r(5.0);
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(4.0)],
        "Relink selected media...",
        Action::RelinkMedia,
        view.studio.selected.is_some() && !view.studio.workflow.media_busy,
        Variant::Secondary,
    )?;
    y += canvas.r(5.0);
    if let Some(label) = &view.studio.workflow.media_label {
        y += canvas.text(label, [left, y], right - left, CAPTION, TEXT_DIMMER, false)? + pad;
    }
    let Some(project) = &view.studio.project else {
        return Ok(());
    };
    let scroll = canvas.begin_scroll("studio_media", [left, y, right, area[3]])?;
    let row = canvas.r(7.0);
    for (index, source) in project.sources.iter().enumerate() {
        let top = y - scroll.offset + index as f32 * row;
        if top + row < y || top > area[3] {
            continue;
        }
        widgets::row(
            canvas,
            view,
            [left, top, right, top + row - canvas.r(0.6)],
            view.studio.selected == Some(source.id),
            Some(MenuAction::Studio(Action::Select(source.id))),
        )?;
        canvas.text_line(
            &source.name,
            [left + pad, top + canvas.r(0.5)],
            right - left - pad * 2.0,
            BODY,
            TEXT,
        )?;
        canvas.text_line(
            &format!(
                "{:?}  {}{}",
                source.kind,
                time_label(source.duration),
                source
                    .frame_rate
                    .map_or_else(String::new, |fps| format!("  {fps} FPS"))
            ),
            [left + pad, top + canvas.r(3.3)],
            right - left - pad * 2.0,
            CAPTION,
            TEXT_DIMMER,
        )?;
    }
    canvas.end_scroll(scroll, project.sources.len() as f32 * row)?;
    Ok(())
}
