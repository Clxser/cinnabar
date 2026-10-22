//! Contextual editing help reads its keyboard bindings from the same registry as input routing.

use super::*;
use launcher::menu::studio::shortcuts::{Command, Scope};

/// Keeps camera navigation guidance separate from document and timeline commands.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    _view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    canvas.text_line("Camera navigation", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(4.2);
    y += canvas.text("Click the 3D replay picture to focus it. Apply saves the inspected pose to your camera shot; navigation itself is temporary. Cameras / Views contains flight speed, optional smooth translation and Level horizon. Players and Path enable editing handles; Navigate keeps the picture clear.", [left,y], right-left, BODY, TEXT_DIMMER, false)? + canvas.r(2.0);
    for &gesture in launcher::menu::studio::NavigationGesture::ALL {
        let chord_width = canvas.r(26.0).min((right - left) * 0.4);
        let height = canvas
            .text(
                gesture.description(),
                [left + chord_width, y],
                right - left - chord_width,
                CAPTION,
                TEXT,
                false,
            )?
            .max(canvas.r(2.7));
        canvas.text_line(
            gesture.chord(),
            [left, y],
            chord_width - canvas.r(1.0),
            CAPTION,
            TEXT_DIMMER,
        )?;
        y += height + canvas.r(1.0);
    }
    y += canvas.r(2.0);
    canvas.text_line("Editing shortcuts", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(4.2);
    y += canvas.text("Timeline shortcuts work when the timeline has focus. Text fields keep normal typing and use Enter to apply or Escape to cancel.", [left,y], right-left, CAPTION, TEXT_DIMMER, false)? + canvas.r(2.0);
    for &command in Command::ALL {
        let spec = command.spec();
        let chord = format!("{}{}", spec.chord.prefix(), spec.key.label());
        let label = format!(
            "{}{}",
            spec.description,
            if spec.scope == Scope::Timeline {
                " (timeline)"
            } else {
                ""
            }
        );
        let chord_width = canvas.r(26.0).min((right - left) * 0.4);
        let height = canvas
            .text(
                &label,
                [left + chord_width, y],
                right - left - chord_width,
                CAPTION,
                TEXT,
                false,
            )?
            .max(canvas.r(2.7));
        canvas.text_line(
            &chord,
            [left, y],
            chord_width - canvas.r(1.0),
            CAPTION,
            TEXT_DIMMER,
        )?;
        y += height + canvas.r(1.0);
    }
    Ok(y + canvas.r(1.0))
}
