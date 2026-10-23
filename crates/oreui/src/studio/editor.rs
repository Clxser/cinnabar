//! A stable editor workspace keeps tools, the viewer, selection properties and tracks separate.

use super::*;
use launcher::menu::studio::EditorPanel;
use recording_studio::MediaTime;

mod inspector;
mod layout;
mod toolbar;
mod toolbox;
mod transport;
mod viewer;

/// Reserves usable media and tracks without replacing the picture when tools are opened in a narrow window.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), PaintError> {
    if view.studio.project.is_none() || area[2] <= area[0] || area[3] <= area[1] {
        return Ok(());
    }
    let pad = canvas.r(0.9);
    let top = toolbar::draw(canvas, view, area)?;
    let width = area[2] - area[0];
    let wide = width >= canvas.r(115.0);
    let workspace = &view.studio.workspace;
    let show_tools = !workspace.preview_only
        && !workspace.media_hidden
        && (wide || workspace.panel == EditorPanel::Media);
    let show_properties = !workspace.preview_only
        && !workspace.inspector_hidden
        && (wide
            || workspace.panel == EditorPanel::Inspector
            || (workspace.panel == EditorPanel::Preview && view.studio.selected.is_some()));
    let tool_width = if show_tools {
        canvas.r(25.0).min(width * if wide { 0.22 } else { 0.32 })
    } else {
        0.0
    };
    let property_width = if show_properties && (!show_tools || wide) {
        canvas.r(29.0).min(width * if wide { 0.27 } else { 0.32 })
    } else {
        0.0
    };
    let left = area[0] + tool_width + if tool_width > 0.0 { pad } else { 0.0 };
    let right = area[2] - property_width - if property_width > 0.0 { pad } else { 0.0 };
    let layout = layout::vertical(
        canvas,
        view,
        [area[0], top.min(area[3]), area[2], area[3]],
        right - left,
    )?;
    let stage = layout.stage;
    if stage[3] > stage[1] {
        if tool_width > 0.0 {
            toolbox::draw(
                canvas,
                view,
                [stage[0], stage[1], stage[0] + tool_width, stage[3]],
            )?;
        }
        if property_width > 0.0 {
            inspector::draw(
                canvas,
                view,
                [stage[2] - property_width, stage[1], stage[2], stage[3]],
            )?;
        }
        preview(canvas, view, [left, stage[1], right, stage[3]])?;
    }
    if workspace.preview_only {
        Ok(())
    } else {
        if view.studio.camera_workspace.graph.open {
            super::graph::draw(canvas, view, layout.timeline)
        } else {
            timeline::draw(canvas, view, layout.timeline)
        }
    }
}
/// Preview overlays and pointer targets share the actual letterboxed image rectangle.
fn preview(canvas: &mut Canvas<'_>, view: &MenuView, area: Bounds) -> Result<(), PaintError> {
    if area[2] <= area[0] || area[3] <= area[1] {
        return Ok(());
    }
    widgets::panel(canvas, area)?;
    let metrics = layout::viewer_metrics(canvas, view, area[2] - area[0])?;
    let pad = metrics.pad;
    // Playback controls retain their measured rows; a short viewer yields picture height first.
    let transport_height = metrics.transport.min(area[3] - area[1]);
    let transport_top = area[3] - transport_height;
    let image_top = viewer::toolbar(canvas, view, [area[0], area[1], area[2], transport_top])?;
    let viewport = [
        area[0] + pad,
        image_top + pad,
        area[2] - pad,
        (image_top + pad + metrics.picture_height(area[3] - area[1])).min(transport_top - pad),
    ];
    if viewport[2] > viewport[0] && viewport[3] > viewport[1] {
        canvas.fill(viewport, [16, 20, 24, 255])?;
        if let Some(icon) = view.studio.preview_icon {
            let content = view
                .studio
                .preview
                .camera
                .as_ref()
                .filter(|camera| camera.interactive)
                .map_or([0.0, 0.0, 1.0, 1.0], |camera| camera.viewport);
            if let Some(image) =
                viewer::image_bounds(icon, viewport, content, view.studio.workspace.preview_fill)
            {
                let clip = canvas.begin_clip(viewport)?;
                canvas.media_in_bounds(icon, image)?;
                let result = super::viewport::draw(canvas, view, image, viewport);
                canvas.end_clip(clip);
                result?;
            }
        } else {
            let caption = if let Some(error) = &view.studio.preview.error {
                error.as_str()
            } else if view.studio.preview.loading {
                "Loading preview..."
            } else if view
                .studio
                .project
                .as_ref()
                .is_none_or(|project| project.sources.is_empty())
            {
                "Import media or open a recording to begin."
            } else {
                "Select a playable recording to preview."
            };
            canvas.text_centred(caption, viewport, CAPTION, TEXT_DIMMER, false)?;
        }
        // A retained successful image must not conceal a later decode or render failure.
        let status = view
            .studio
            .preview
            .error
            .as_deref()
            .or(view.studio.audio.error.as_deref());
        if let Some(status) = status {
            let height = canvas.r(3.2).min(viewport[3] - viewport[1]);
            canvas.fill(
                [viewport[0], viewport[1], viewport[2], viewport[1] + height],
                [20, 25, 30, 235],
            )?;
            let recovery = view.studio.camera_workspace.view.is_some();
            let right = viewport[2] - if recovery { canvas.r(14.0) } else { pad };
            canvas.text_line(
                status,
                [viewport[0] + pad, viewport[1] + canvas.r(0.5)],
                (right - viewport[0] - pad).max(0.0),
                CAPTION,
                TEXT_DIMMER,
            )?;
            if recovery {
                control(
                    canvas,
                    view,
                    [right, viewport[1], viewport[2], viewport[1] + height],
                    "Reset view",
                    Action::ResetCameraView,
                    true,
                    Variant::Neutral,
                )?;
            }
        }
    }
    transport::draw(
        canvas,
        view,
        [area[0] + pad, transport_top, area[2] - pad, area[3] - pad],
    )
}
