//! The studio library draws real local entries and keeps empty/loading/error states actionable.

use super::*;
use launcher::menu::{MenuField, studio::library::LibraryAction};
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
    let import_left = left + canvas.r(32.4);
    if right - import_left > canvas.r(14.0) {
        control(
            canvas,
            view,
            [
                import_left,
                button_top,
                right.min(import_left + canvas.r(20.0)),
                button_top + canvas.r(4.8),
            ],
            if view.studio.workflow.media_busy {
                "Cancel import"
            } else {
                "Import media..."
            },
            if view.studio.workflow.media_busy {
                Action::CancelMedia
            } else {
                Action::ImportMedia
            },
            true,
            Variant::Secondary,
        )?;
    }
    let content_top = button_top + canvas.r(7.2);
    let mut content_top = content_top;
    if right - left > canvas.r(20.0) && content_top + canvas.r(3.8) <= area[3] - pad {
        control(
            canvas,
            view,
            [
                left,
                content_top,
                left + canvas.r(20.0),
                content_top + canvas.r(3.8),
            ],
            &if view.studio.workflow.recovery.entries.is_empty() {
                "Project recovery".to_owned()
            } else {
                format!(
                    "Recover projects ({})",
                    view.studio.workflow.recovery.entries.len()
                )
            },
            Action::ShowRecovery,
            !view.studio.workflow.recovery.busy,
            Variant::Neutral,
        )?;
        content_top += canvas.r(4.8);
    }
    if content_top >= area[3] - pad {
        return Ok(());
    }
    content_top = query_controls(canvas, view, [left, content_top, right, area[3] - pad])?;
    if content_top >= area[3] - pad {
        return Ok(());
    }
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
            if view.studio.workspace.library.search.is_empty() {
                "Your next video starts here."
            } else {
                "No matching recordings or projects."
            },
            [left, content_top + canvas.r(3.2)],
            right - left,
            HEADER5,
            TEXT,
        )?;
        canvas.text_line(
            if view.studio.workspace.library.search.is_empty() {
                "Recordings and saved projects appear in this library."
            } else {
                "Try a shorter search or clear the filter."
            },
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
                (!entry.interrupted || !view.studio.capture.phase.busy()).then_some(
                    MenuAction::Studio(Action::Open {
                        generation: view.studio.workspace.library.generation,
                        index,
                    }),
                ),
            )?;
            let thumbnail = view
                .studio
                .workflow
                .thumbnail_slots
                .get(index)
                .copied()
                .flatten()
                .zip(view.studio.workflow.thumbnail_icon)
                .map(|(uv, mut icon)| {
                    icon.uv = uv;
                    icon
                });
            let image_right = left + canvas.r(11.6);
            if let Some(icon) = thumbnail {
                canvas.fitted_media(
                    icon,
                    [
                        left + canvas.r(0.5),
                        top + canvas.r(0.5),
                        image_right,
                        bounds[3] - canvas.r(0.5),
                    ],
                )?;
            }
            let text_left = if thumbnail.is_some() {
                image_right + pad
            } else {
                left + pad
            };
            canvas.text_line(
                &entry.name,
                [text_left, top + canvas.r(0.8)],
                right - text_left - pad,
                BODY,
                TEXT,
            )?;
            let kind = match entry.kind {
                LibraryKind::Project => "Project",
                LibraryKind::Recording if entry.interrupted => {
                    "Interrupted take - open to attempt video recovery"
                }
                LibraryKind::Recording => "Recording",
            };
            canvas.text_line(
                kind,
                [text_left, top + canvas.r(3.4)],
                right - text_left - pad,
                CAPTION,
                TEXT_DIMMER,
            )?;
        }
        canvas.end_scroll(scroll, view.studio.library.len() as f32 * row_height)?;
    }
    if view.studio.library_truncated {
        canvas.text_line(
            "Library limit reached. Narrow the search; more files may remain in the folder.",
            [left, area[3] - canvas.r(3.6)],
            right - left,
            CAPTION,
            TEXT_DIMMER,
        )?;
    }
    Ok(())
}

/// Search has an explicit Apply boundary, so typing never triggers a filesystem scan per keystroke.
fn query_controls(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    let [left, top, right, bottom] = area;
    let row = canvas.r(3.8);
    let gap = canvas.r(0.8);
    if top + row > bottom {
        return Ok(bottom);
    }
    let enabled = !view.studio.library_loading;
    let state = &view.studio.workspace.library;
    let draft = state.draft.as_deref();
    let split = right - left < canvas.r(65.0);
    let tools = canvas.r(35.0).min((right - left).max(0.0));
    let field_right = if split { right } else { right - tools - gap };
    widgets::text_field(
        canvas,
        view,
        [left, top, field_right, top + row],
        draft.unwrap_or(&state.search),
        "Search recordings and projects",
        view.field == Some(MenuField::StudioLibrarySearch),
        enabled.then_some(MenuAction::Studio(Action::LibraryTools(
            LibraryAction::Search,
        ))),
    )?;
    let tools_top = if split { top + row + gap } else { top };
    if tools_top + row > bottom {
        return Ok(top + row + gap);
    }
    let mut x = if split { left } else { field_right + gap };
    for (label, action, width, active) in [
        ("Search", LibraryAction::ApplySearch, 9.0, draft.is_some()),
        (
            "Clear",
            LibraryAction::ClearSearch,
            8.0,
            !state.search.is_empty() || draft.is_some(),
        ),
        (state.sort.label(), LibraryAction::CycleSort, 16.0, true),
    ] {
        let edge = (x + canvas.r(width)).min(right);
        if edge - x >= canvas.r(6.0) {
            control(
                canvas,
                view,
                [x, tools_top, edge, tools_top + row],
                label,
                Action::LibraryTools(action),
                enabled && active,
                Variant::Secondary,
            )?;
        }
        x = edge + gap;
    }
    Ok(tools_top + row + canvas.r(1.6))
}
