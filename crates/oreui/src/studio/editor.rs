//! A stable editor workspace keeps tools, the viewer, selection properties and tracks separate.

use super::*;
use launcher::menu::studio::EditorPanel;
use recording_studio::MediaTime;

mod inspector;
mod toolbar;
mod toolbox;
mod transport;
mod viewer;

/// Reserves usable preview and track regions without scrolling the entire editor off screen.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), PaintError> {
    if view.studio.project.is_none() || area[2] <= area[0] || area[3] <= area[1] {
        return Ok(());
    }
    let pad = canvas.r(0.9);
    let mut top = toolbar::draw(canvas, view, area)?;
    let width = area[2] - area[0];
    let wide = width >= canvas.r(115.0);
    let medium = width >= canvas.r(78.0);
    if !medium && !view.studio.workspace.preview_only {
        let gap = canvas.r(0.5);
        let button = (width - gap * 2.0) / 3.0;
        for (index, (label, panel)) in [
            ("Preview", EditorPanel::Preview),
            ("Tools", EditorPanel::Media),
            ("Properties", EditorPanel::Inspector),
        ]
        .into_iter()
        .enumerate()
        {
            let x = area[0] + index as f32 * (button + gap);
            control(
                canvas,
                view,
                [x, top, x + button, top + canvas.r(3.4)],
                label,
                Action::Panel(panel),
                true,
                if view.studio.workspace.panel == panel {
                    Variant::Primary
                } else {
                    Variant::Secondary
                },
            )?;
        }
        top += canvas.r(4.3);
    }
    let available = (area[3] - top).max(0.0);
    if available <= canvas.r(2.0) {
        return Ok(());
    }
    let minimum = canvas.r(17.0).min(available * 0.45);
    let maximum = (available - canvas.r(13.0)).max(minimum);
    let timeline_height = if view.studio.workspace.preview_only {
        0.0
    } else {
        (available * view.studio.workspace.timeline_size.fraction()).clamp(minimum, maximum)
    };
    let timeline_top = (area[3] - timeline_height).max(top);
    let stage = [area[0], top, area[2], (timeline_top - pad).max(top)];
    if stage[3] > stage[1] {
        if !medium && !view.studio.workspace.preview_only {
            match view.studio.workspace.panel {
                EditorPanel::Media => toolbox::draw(canvas, view, stage)?,
                EditorPanel::Inspector => inspector::draw(canvas, view, stage)?,
                EditorPanel::Preview => preview(canvas, view, stage)?,
            }
        } else {
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
                canvas.r(26.0).min(width * 0.25)
            } else {
                0.0
            };
            let property_width = if show_properties && (!show_tools || wide) {
                canvas.r(31.0).min(width * 0.32)
            } else {
                0.0
            };
            let viewer = [
                stage[0] + tool_width + if tool_width > 0.0 { pad } else { 0.0 },
                stage[1],
                stage[2] - property_width - if property_width > 0.0 { pad } else { 0.0 },
                stage[3],
            ];
            if tool_width > 0.0 {
                toolbox::draw(
                    canvas,
                    view,
                    [stage[0], top, stage[0] + tool_width, stage[3]],
                )?;
            }
            if property_width > 0.0 {
                inspector::draw(
                    canvas,
                    view,
                    [stage[2] - property_width, top, stage[2], stage[3]],
                )?;
            }
            preview(canvas, view, viewer)?;
        }
    }
    if view.studio.workspace.preview_only {
        Ok(())
    } else {
        timeline::draw(canvas, view, [area[0], timeline_top, area[2], area[3]])
    }
}

/// Preview overlays and pointer targets share the actual letterboxed image rectangle.
fn preview(canvas: &mut Canvas<'_>, view: &MenuView, area: Bounds) -> Result<(), PaintError> {
    if area[2] <= area[0] || area[3] <= area[1] {
        return Ok(());
    }
    widgets::panel(canvas, area)?;
    let pad = canvas.r(0.8);
    let transport_height = canvas.r(8.4).min((area[3] - area[1]) * 0.45);
    let transport_top = area[3] - transport_height;
    let mut image_top = viewer::toolbar(canvas, view, [area[0], area[1], area[2], transport_top])?;
    if let Some(notice) = &view.studio.preview.notice {
        let notice_height = canvas.r(2.5);
        if transport_top - image_top > notice_height + canvas.r(6.0) {
            canvas.text_line(
                notice,
                [area[0] + pad, image_top],
                (area[2] - area[0] - pad * 2.0).max(0.0),
                CAPTION,
                TEXT_DIMMER,
            )?;
            image_top += notice_height;
        }
    }
    let viewport = [
        area[0] + pad,
        image_top + pad,
        area[2] - pad,
        transport_top - pad,
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
