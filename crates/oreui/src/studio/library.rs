//! The studio library draws real local entries and keeps empty/loading/error states actionable.

use super::*;
use recording_studio::storage::LibraryKind;

pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), PaintError> {
    let pad = canvas.r(2.0);
    let left = area[0] + pad;
    let right = area[2] - pad;
    let top = area[1] + pad;
    widgets::panel(canvas, area)?;
    canvas.text_line("Your studio", [left, top], right - left, HEADER5, TEXT)?;
    let button_top = top + canvas.r(4.0);
    control(
        canvas,
        view,
        [
            left,
            button_top,
            left + canvas.r(18.0),
            button_top + canvas.r(4.8),
        ],
        "New project",
        Action::NewProject,
        true,
        Variant::Primary,
    )?;
    control(
        canvas,
        view,
        [
            left + canvas.r(19.2),
            button_top,
            left + canvas.r(31.2),
            button_top + canvas.r(4.8),
        ],
        "Refresh",
        Action::Refresh,
        !view.studio.library_loading,
        Variant::Secondary,
    )?;
    let content_top = button_top + canvas.r(7.2);
    if view.studio.library_loading {
        canvas.text_line(
            "Loading your studio...",
            [left, content_top],
            right - left,
            BODY,
            TEXT_DIMMER,
        )?;
    } else if view.studio.library.is_empty() {
        canvas.text_line(
            "Your next video starts here.",
            [left, content_top + canvas.r(3.2)],
            right - left,
            HEADER5,
            TEXT,
        )?;
        canvas.text_line(
            "Recordings and saved projects appear in this library.",
            [left, content_top + canvas.r(7.2)],
            right - left,
            BODY,
            TEXT_DIMMER,
        )?;
    } else {
        let scroll =
            canvas.begin_scroll("studio_library", [left, content_top, right, area[3] - pad])?;
        let start = content_top - scroll.offset;
        let row_height = canvas.r(7.2);
        for (index, entry) in view.studio.library.iter().enumerate() {
            let top = start + index as f32 * row_height;
            let bounds = [left, top, right, top + row_height - canvas.r(0.8)];
            if bounds[3] < content_top || bounds[1] > area[3] {
                continue;
            }
            widgets::row(
                canvas,
                view,
                bounds,
                false,
                Some(MenuAction::Studio(Action::Open(index))),
            )?;
            canvas.text_line(
                &entry.name,
                [left + pad, top + canvas.r(0.8)],
                right - left - pad * 2.0,
                BODY,
                TEXT,
            )?;
            let kind = match entry.kind {
                LibraryKind::Project => "Project",
                LibraryKind::Recording => "Recording",
            };
            canvas.text_line(
                kind,
                [left + pad, top + canvas.r(3.4)],
                right - left - pad * 2.0,
                CAPTION,
                TEXT_DIMMER,
            )?;
        }
        canvas.end_scroll(scroll, view.studio.library.len() as f32 * row_height)?;
    }
    if view.studio.library_truncated {
        canvas.text_line(
            "Showing recent items. More projects remain in your studio folder.",
            [left, area[3] - canvas.r(3.6)],
            right - left,
            CAPTION,
            TEXT_DIMMER,
        )?;
    }
    Ok(())
}
