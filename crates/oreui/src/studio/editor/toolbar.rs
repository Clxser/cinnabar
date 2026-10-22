//! A compact tool strip keeps the viewer dominant and leaves occasional settings in Preview settings.

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
    let tab_width = canvas.r(8.2);
    let tools_width = Toolbox::ALL.len() as f32 * tab_width + (Toolbox::ALL.len() - 1) as f32 * gap;
    let controls = [
        (
            "Properties",
            11.5,
            WorkspaceAction::ToggleInspector,
            !view.studio.workspace.inspector_hidden && !view.studio.workspace.preview_only,
        ),
        (
            view.studio.workspace.layout.label(),
            13.0,
            WorkspaceAction::CycleLayout,
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
    ];
    let controls_width = controls
        .iter()
        .map(|(_, width, _, _)| canvas.r(*width))
        .sum::<f32>()
        + gap * (controls.len() - 1) as f32;
    let wrap = area[2] - area[0] < tools_width + controls_width + gap * 2.0;
    let mut x = area[0];
    let mut tools_top = area[1];
    for tool in Toolbox::ALL.iter().copied() {
        if x > area[0] && x + tab_width > area[2] {
            x = area[0];
            tools_top += height + gap;
        }
        control(
            canvas,
            view,
            [
                x,
                tools_top,
                (x + tab_width).min(area[2]),
                tools_top + height,
            ],
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
        x += tab_width + gap;
    }
    let mut top = if wrap {
        tools_top + height + gap
    } else {
        area[1]
    };
    let mut x = if wrap {
        area[0]
    } else {
        area[2] - controls_width
    };
    for (label, width, action, active) in controls {
        let width = canvas.r(width);
        if x > area[0] && x + width > area[2] + 0.1 {
            x = area[0];
            top += height + gap;
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
