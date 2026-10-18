//! Viewer controls and aspect-preserving framing share one geometry for drawing and pointer projection.

use super::*;
use launcher::menu::studio::{ViewportTool, WorkspaceAction};

/// Navigation tools are explicit; normal playback never paints subject or path editing markers.
pub(super) fn toolbar(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    let pad = canvas.r(0.6);
    let height = canvas.r(3.4);
    if area[3] - area[1] < height + canvas.r(6.0) {
        return Ok(area[1]);
    }
    let top = area[1] + pad;
    let mut x = area[0] + pad;
    let right = area[2] - pad;
    let fit_width = canvas.r(8.2).min((right - x).max(0.0));
    let tools_right = right - fit_width - pad;
    if view
        .studio
        .preview
        .camera
        .as_ref()
        .is_some_and(|camera| camera.interactive)
    {
        for tool in ViewportTool::ALL.iter().copied() {
            let width = canvas.r(7.6);
            if x + width > tools_right {
                break;
            }
            control(
                canvas,
                view,
                [x, top, x + width, top + height],
                tool.label(),
                Action::Workspace(WorkspaceAction::ViewportTool(tool)),
                true,
                if view.studio.workspace.viewport_tool == tool {
                    Variant::Primary
                } else {
                    Variant::Neutral
                },
            )?;
            x += width + pad;
        }
    }
    control(
        canvas,
        view,
        [right - fit_width, top, right, top + height],
        if view.studio.workspace.preview_fill {
            "Fill view"
        } else {
            "Fit view"
        },
        Action::Workspace(WorkspaceAction::TogglePreviewFill),
        true,
        Variant::Secondary,
    )?;
    let quality_width = canvas.r(10.4);
    if x + quality_width <= tools_right {
        control(
            canvas,
            view,
            [x, top, x + quality_width, top + height],
            view.studio.workspace.preview_quality.label(),
            Action::Workspace(WorkspaceAction::CyclePreviewQuality),
            true,
            Variant::Neutral,
        )?;
        x += quality_width + pad;
    }
    let mut end = top + height;
    if view.studio.camera_workspace.view.is_some() {
        let width = canvas.r(7.6);
        let mut y = top;
        if x + (width + pad) * 2.0 > tools_right {
            x = area[0] + pad;
            y += height + pad;
        }
        for (label, action, enabled) in
            super::super::viewport::view_actions(view.studio.preview.ready)
        {
            if x + width > right {
                break;
            }
            control(
                canvas,
                view,
                [x, y, x + width, y + height],
                label,
                action,
                enabled,
                Variant::Secondary,
            )?;
            x += width + pad;
        }
        end = y + height;
        if view.studio.workspace.viewport_tool == ViewportTool::Path
            && x + canvas.r(11.0) <= tools_right
        {
            control(
                canvas,
                view,
                [x, y, x + canvas.r(11.0), y + height],
                "Auto handles",
                Action::ResetCameraHandles,
                view.studio.camera_workspace.key.is_some(),
                Variant::Neutral,
            )?;
        }
    }
    Ok(end + pad)
}

/// Fits the meaningful native image, excluding composition bars, then maps the entire texture consistently.
pub(super) fn image_bounds(
    icon: ui::IconRef,
    viewport: Bounds,
    content: [f64; 4],
    fill: bool,
) -> Option<Bounds> {
    if !content
        .iter()
        .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
        || content[2] <= content[0]
        || content[3] <= content[1]
    {
        return None;
    }
    let full = [
        f64::from(icon.uv[2].checked_sub(icon.uv[0])?),
        f64::from(icon.uv[3].checked_sub(icon.uv[1])?),
    ];
    if full.contains(&0.0)
        || !viewport.iter().all(|v| v.is_finite())
        || viewport[2] <= viewport[0]
        || viewport[3] <= viewport[1]
    {
        return None;
    }
    let image = [
        full[0] * (content[2] - content[0]),
        full[1] * (content[3] - content[1]),
    ];
    let scales = [
        f64::from(viewport[2] - viewport[0]) / image[0],
        f64::from(viewport[3] - viewport[1]) / image[1],
    ];
    let scale = if fill {
        scales[0].max(scales[1])
    } else {
        scales[0].min(scales[1])
    };
    let left = (f64::from(viewport[0] + viewport[2]) - image[0] * scale) * 0.5
        - full[0] * content[0] * scale;
    let top = (f64::from(viewport[1] + viewport[3]) - image[1] * scale) * 0.5
        - full[1] * content[1] * scale;
    Some([
        left as f32,
        top as f32,
        (left + full[0] * scale) as f32,
        (top + full[1] * scale) as f32,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fit_removes_composition_bars_and_fill_crops_without_distorting_projection() {
        let icon = ui::IconRef {
            page: 0,
            uv: [0, 0, 1000, 1000],
            glint: false,
        };
        let native = [0.0, 0.25, 1.0, 0.75];
        assert_eq!(
            image_bounds(icon, [0.0, 0.0, 800.0, 400.0], native, false),
            Some([0.0, -200.0, 800.0, 600.0])
        );
        assert_eq!(
            image_bounds(icon, [0.0, 0.0, 400.0, 400.0], native, true),
            Some([-200.0, -200.0, 600.0, 600.0])
        );
        assert_eq!(
            image_bounds(icon, [0.0, 0.0, 400.0, 400.0], [0.0; 4], false),
            None
        );
    }
}
