//! Explicit seconds distinguish the saved timeline position from the clip's retained source handles.

use super::*;
use launcher::menu::studio::ClipTimeField;

/// Typed source trims keep Position; ordinary validation owns source bounds, ripple and overlap errors.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    clip: &Clip,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    canvas.text_line("CLIP TIMING", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(4.0);
    for &field in ClipTimeField::ALL {
        canvas.text_line(field.label(), [left, y], right - left, CAPTION, TEXT_DIMMER)?;
        y += canvas.r(2.8);
        let value = format!("{:.6}", field.value(clip).seconds());
        let draft = view
            .studio
            .clip_draft
            .as_ref()
            .filter(|(id, parameter, _)| *id == clip.id && *parameter == field)
            .map(|(_, _, text)| text.as_ref());
        widgets::text_field(
            canvas,
            view,
            [left, y, right, y + canvas.r(3.8)],
            draft.unwrap_or(&value),
            field.label(),
            draft.is_some() && view.field == Some(launcher::menu::MenuField::StudioClip(field)),
            Some(MenuAction::Studio(Action::EditClipTime(field))),
        )?;
        y += canvas.r(4.8);
        if draft.is_some() {
            let width = (right - left - canvas.r(0.8)) * 0.5;
            for (index, (label, action)) in [
                ("Apply", Action::ApplyClipTime),
                ("Cancel", Action::CancelClipTime),
            ]
            .into_iter()
            .enumerate()
            {
                let x = left + index as f32 * (width + canvas.r(0.8));
                control(
                    canvas,
                    view,
                    [x, y, x + width, y + canvas.r(3.8)],
                    label,
                    action,
                    true,
                    Variant::Secondary,
                )?;
            }
            y += canvas.r(4.8);
        }
    }
    Ok(y + canvas.r(1.2))
}
