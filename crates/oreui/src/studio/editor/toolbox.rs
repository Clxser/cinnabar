//! Creation tools stay in one clearly named library rather than below every property inspector.

use super::*;
use launcher::menu::studio::Toolbox;

#[cfg(test)]
mod tests;

/// Only the active tool's content scrolls; media owns its own virtualized source list.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), PaintError> {
    widgets::panel(canvas, area)?;
    let pad = canvas.r(1.2);
    let mut inner = [area[0] + pad, area[1] + pad, area[2] - pad, area[3] - pad];
    if inner[2] <= inner[0] || inner[3] <= inner[1] {
        return Ok(());
    }
    let tool = view.studio.workspace.toolbox;
    if matches!(tool, Toolbox::Media | Toolbox::Audio) {
        return super::super::media::draw_filtered(canvas, view, inner, tool == Toolbox::Audio);
    }
    let section = match tool {
        Toolbox::Cameras => {
            let selected = view.studio.camera_workspace.tools;
            inner[1] = section_tabs(
                canvas,
                view,
                inner,
                launcher::menu::studio::CameraTools::ALL
                    .iter()
                    .copied()
                    .map(|section| {
                        (
                            section.label(),
                            Action::CameraViews(
                                launcher::menu::studio::CameraViewsAction::Section(section),
                            ),
                            section == selected,
                        )
                    }),
            )?;
            selected.label()
        }
        Toolbox::Creative => {
            let selected = view.studio.workflow.creative.section;
            inner[1] = section_tabs(
                canvas,
                view,
                inner,
                launcher::menu::studio::AssistSection::ALL
                    .iter()
                    .copied()
                    .map(|section| {
                        (
                            section.label(),
                            Action::Creative(
                                launcher::menu::studio::creative::CreativeAction::Section(section),
                            ),
                            section == selected,
                        )
                    }),
            )?;
            selected.label()
        }
        _ => "",
    };
    let previous = canvas.settings_scrollbars;
    canvas.settings_scrollbars = true;
    let key = format!("studio_tools_{}_{}", tool.label(), section);
    let scroll = canvas.begin_scroll(&key, inner)?;
    let top = inner[1] - scroll.offset;
    let content = canvas.scroll_content_bounds([inner[0], top, inner[2], inner[3]]);
    let bottom = match tool {
        Toolbox::Cameras => super::super::camera::creation(canvas, view, content)?,
        Toolbox::Effects => super::super::effects::creation(canvas, view, content)?,
        Toolbox::Creative => super::super::creative::draw(canvas, view, content)?,
        Toolbox::Text => {
            canvas.text_line(
                "Text",
                [content[0], top],
                content[2] - content[0],
                HEADER5,
                TEXT,
            )?;
            let y = top + canvas.r(4.2);
            control(
                canvas,
                view,
                [content[0], y, content[2], y + canvas.r(4.0)],
                "Add title",
                Action::AddTitle,
                true,
                Variant::Primary,
            )?;
            y + canvas.r(5.6) + canvas.text("Add a title at the playhead, then select it to edit its text, position and fades.", [content[0], y + canvas.r(5.6)], content[2] - content[0], CAPTION, TEXT_DIMMER, false)?
        }
        Toolbox::Media | Toolbox::Audio => unreachable!(),
    };
    let result = canvas.end_scroll(scroll, (bottom - top).max(0.0));
    canvas.settings_scrollbars = previous;
    result
}

/// Section tabs stay outside scrolling content and share labels with the action owner.
fn section_tabs(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    sections: impl ExactSizeIterator<Item = (&'static str, Action, bool)>,
) -> Result<f32, PaintError> {
    let gap = canvas.r(0.4);
    let width = (area[2] - area[0] - gap * sections.len().saturating_sub(1) as f32)
        / sections.len().max(1) as f32;
    for (index, (label, action, selected)) in sections.enumerate() {
        let left = area[0] + index as f32 * (width + gap);
        control(
            canvas,
            view,
            [left, area[1], left + width, area[1] + canvas.r(3.5)],
            label,
            action,
            true,
            if selected {
                Variant::Primary
            } else {
                Variant::Secondary
            },
        )?;
    }
    Ok(area[1] + canvas.r(4.4))
}
