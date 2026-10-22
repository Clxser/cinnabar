//! Viewer controls and aspect-preserving framing share one geometry for drawing and pointer projection.

use super::*;
use launcher::menu::studio::{ViewportTool, WorkspaceAction};

/// Measures the exact controls which the viewer will paint, including wrapping Apply and Reset.
pub(super) fn required_height(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    width: f32,
) -> Result<f32, PaintError> {
    walk(canvas, view, [0.0, 0.0, width, 0.0], false)
}

/// Navigation tools are explicit; normal playback never paints subject or path editing markers.
pub(super) fn toolbar(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    walk(canvas, view, area, true)
}

/// Measurement and painting use one bounded control iterator and identical row packing.
fn walk(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    paint: bool,
) -> Result<f32, PaintError> {
    let pad = canvas.r(0.6);
    let height = canvas.r(3.4);
    let left = area[0] + pad;
    let right = area[2] - pad;
    if right <= left {
        return Ok(area[1]);
    }
    let workspace = &view.studio.workspace;
    let interactive = view
        .studio
        .preview
        .camera
        .as_ref()
        .is_some_and(|camera| camera.interactive);
    let inspected = view.studio.camera_workspace.view.is_some();
    let controls = ViewportTool::ALL
        .iter()
        .copied()
        .filter(|_| interactive)
        .map(|tool| {
            (
                tool.label(),
                Action::Workspace(WorkspaceAction::ViewportTool(tool)),
                true,
                if workspace.viewport_tool == tool {
                    Variant::Primary
                } else {
                    Variant::Neutral
                },
            )
        })
        .chain(std::iter::once((
            if workspace.preview_fill {
                "Fill view"
            } else {
                "Fit view"
            },
            Action::Workspace(WorkspaceAction::TogglePreviewFill),
            true,
            Variant::Secondary,
        )))
        .chain(std::iter::once((
            workspace.preview_quality.label(),
            Action::Workspace(WorkspaceAction::CyclePreviewQuality),
            true,
            Variant::Neutral,
        )))
        .chain(
            super::super::viewport::view_actions(view.studio.preview.ready)
                .into_iter()
                .filter(|_| inspected)
                .map(|(label, action, enabled)| (label, action, enabled, Variant::Secondary)),
        )
        .chain(
            std::iter::once((
                "Auto handles",
                Action::ResetCameraHandles,
                view.studio.camera_workspace.key.is_some(),
                Variant::Neutral,
            ))
            .filter(|_| inspected && workspace.viewport_tool == ViewportTool::Path),
        );
    let mut x = left;
    let mut y = area[1] + pad;
    for (label, action, enabled, variant) in controls {
        let width = (canvas.measure(label, crate::theme::SECONDARY_BUTTON)? + canvas.r(1.6))
            .max(canvas.r(3.0))
            .min(right - left);
        if x > left && x + width > right + 0.1 {
            x = left;
            y += height + pad;
        }
        if paint && y + height <= area[3] + 0.1 {
            control(
                canvas,
                view,
                [x, y, x + width, y + height],
                label,
                action,
                enabled,
                variant,
            )?;
        }
        x += width + pad;
    }
    Ok(y + height + pad)
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
