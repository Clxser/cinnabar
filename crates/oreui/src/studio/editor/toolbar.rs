//! Tools, panel visibility and workspace density stay directly reachable above the viewer.

use super::*;
use launcher::menu::studio::{Toolbox, WorkspaceAction};

/// Wraps workspace controls only when their real combined widths no longer fit.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    let gap = canvas.r(0.5);
    let height = canvas.r(3.6);
    let tools_width =
        Toolbox::ALL.len() as f32 * canvas.r(8.2) + (Toolbox::ALL.len() - 1) as f32 * gap;
    let controls = [
        (
            "Properties",
            11.5,
            WorkspaceAction::ToggleInspector,
            !view.studio.workspace.inspector_hidden && !view.studio.workspace.preview_only,
        ),
        (
            view.studio.workspace.timeline_size.label(),
            13.0,
            WorkspaceAction::TimelineSize,
            false,
        ),
        (
            if view.studio.workspace.preview_only {
                "Show panels"
            } else {
                "Focus view"
            },
            12.0,
            WorkspaceAction::PreviewOnly,
            view.studio.workspace.preview_only,
        ),
        (
            view.studio.workspace.density.label(),
            14.5,
            WorkspaceAction::Density,
            false,
        ),
    ];
    let controls_width = controls
        .iter()
        .map(|(_, width, _, _)| canvas.r(*width))
        .sum::<f32>()
        + gap * (controls.len() - 1) as f32;
    let wrap = area[2] - area[0] < tools_width + controls_width + gap * 2.0;
    let mut x = area[0];
    for tool in Toolbox::ALL.iter().copied() {
        control(
            canvas,
            view,
            [x, area[1], x + canvas.r(8.2), area[1] + height],
            tool.label(),
            Action::Workspace(WorkspaceAction::Toolbox(tool)),
            true,
            if view.studio.workspace.toolbox == tool
                && !view.studio.workspace.media_hidden
                && !view.studio.workspace.preview_only
            {
                Variant::Primary
            } else {
                Variant::Neutral
            },
        )?;
        x += canvas.r(8.2) + gap;
    }
    let top = area[1] + if wrap { height + gap } else { 0.0 };
    let mut x = if wrap {
        area[0]
    } else {
        area[2] - controls_width
    };
    for (label, width, action, active) in controls {
        let width = canvas.r(width);
        if x + width > area[2] + 0.1 {
            break;
        }
        control(
            canvas,
            view,
            [x, top, (x + width).min(area[2]), top + height],
            label,
            Action::Workspace(action),
            true,
            if active {
                Variant::Secondary
            } else {
                Variant::Neutral
            },
        )?;
        x += width + gap;
    }
    Ok(top + height + canvas.r(0.9))
}
