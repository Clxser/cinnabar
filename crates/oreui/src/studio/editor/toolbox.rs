//! Creation tools stay in one clearly named library rather than below every property inspector.

use super::*;
use launcher::menu::studio::Toolbox;

/// Only the active tool's content scrolls; media owns its own virtualized source list.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), PaintError> {
    widgets::panel(canvas, area)?;
    let pad = canvas.r(1.2);
    let inner = [area[0] + pad, area[1] + pad, area[2] - pad, area[3] - pad];
    if inner[2] <= inner[0] || inner[3] <= inner[1] {
        return Ok(());
    }
    let tool = view.studio.workspace.toolbox;
    if matches!(tool, Toolbox::Media | Toolbox::Audio) {
        return super::super::media::draw_filtered(canvas, view, inner, tool == Toolbox::Audio);
    }
    let previous = canvas.settings_scrollbars;
    canvas.settings_scrollbars = true;
    let key = format!("studio_tools_{}", tool.label());
    let scroll = canvas.begin_scroll(&key, inner)?;
    let top = inner[1] - scroll.offset;
    let content = [inner[0], top, inner[2], inner[3]];
    let bottom = match tool {
        Toolbox::Cameras => super::super::camera::creation(canvas, view, content)?,
        Toolbox::Effects => super::super::effects::creation(canvas, view, content)?,
        Toolbox::Text => {
            canvas.text_line("Text", [inner[0], top], inner[2] - inner[0], HEADER5, TEXT)?;
            let y = top + canvas.r(4.2);
            control(
                canvas,
                view,
                [inner[0], y, inner[2], y + canvas.r(4.0)],
                "Add title",
                Action::AddTitle,
                true,
                Variant::Primary,
            )?;
            y + canvas.r(5.6) + canvas.text("Add a title at the playhead, then select it to edit its text, position and fades.", [inner[0], y + canvas.r(5.6)], inner[2] - inner[0], CAPTION, TEXT_DIMMER, false)?
        }
        Toolbox::Media | Toolbox::Audio => unreachable!(),
    };
    let result = canvas.end_scroll(scroll, (bottom - top).max(0.0));
    canvas.settings_scrollbars = previous;
    result
}
