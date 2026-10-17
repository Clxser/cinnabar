//! Recording and export settings use the studio's existing native scroll and control primitives.

use super::*;
use recording_studio::settings::{codec_label, encoder_label};

/// Keeps every output choice reachable on small windows without covering the transport with a modal.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), PaintError> {
    widgets::panel(canvas, area)?;
    let pad = canvas.r(2.0);
    let [left, top, right, bottom] = [area[0] + pad, area[1] + pad, area[2] - pad, area[3] - pad];
    let scroll = canvas.begin_scroll("studio_settings", [left, top, right, bottom])?;
    let mut y = top - scroll.offset;
    canvas.text_line(
        "Recording and export",
        [left, y],
        right - left,
        HEADER5,
        TEXT,
    )?;
    y += canvas.r(4.8);
    let capture = view.studio.workflow.capture_settings;
    row(
        canvas,
        view,
        [left, y, right, bottom],
        &format!("Recording FPS: {}", capture.fps),
        Action::CycleCaptureFps,
        !view.studio.capture.phase.busy(),
    )?;
    y += canvas.r(5.0);
    row(
        canvas,
        view,
        [left, y, right, bottom],
        &format!("Recording size: {}", capture.size.label()),
        Action::CycleCaptureSize,
        !view.studio.capture.phase.busy(),
    )?;
    y += canvas.r(5.0);
    row(
        canvas,
        view,
        [left, y, right, bottom],
        if capture.hand {
            "Record first-person hand: On"
        } else {
            "Record first-person hand: Off"
        },
        Action::ToggleCaptureHand,
        !view.studio.capture.phase.busy(),
    )?;
    y += canvas.r(5.0);
    row(
        canvas,
        view,
        [left, y, right, bottom],
        if capture.hud {
            "Record HUD and chat: On"
        } else {
            "Record HUD and chat: Off"
        },
        Action::ToggleCaptureHud,
        !view.studio.capture.phase.busy(),
    )?;
    y += canvas.r(5.0);
    y += canvas.text("120/240 FPS can retain more motion for slow motion. Recording size uses GPU scaling before readback and never upscales. Capture follows actual game frames; missed slots repeat the previous frame and are reported. Changes apply to the next take.", [left, y], right - left, CAPTION, TEXT_DIMMER, false)? + canvas.r(3.0);
    if let Some(project) = &view.studio.project {
        let settings = &project.export;
        let enabled = !view.studio.loading;
        for (label, action) in [
            (
                format!("Output size: {} x {}", settings.size[0], settings.size[1]),
                Action::CycleExportSize,
            ),
            (
                format!("Output FPS: {}", settings.fps),
                Action::CycleExportFps,
            ),
            (
                format!("Format: {} (MP4)", codec_label(settings.codec)),
                Action::CycleExportCodec,
            ),
            (
                format!("Encoder: {}", encoder_label(settings.encoder)),
                Action::CycleExportEncoder,
            ),
            (
                format!(
                    "Export audio: {}",
                    if settings.include_audio { "On" } else { "Off" }
                ),
                Action::ToggleExportAudio,
            ),
        ] {
            row(
                canvas,
                view,
                [left, y, right, bottom],
                &label,
                action,
                enabled,
            )?;
            y += canvas.r(5.0);
        }
        let button = canvas.r(5.6);
        canvas.text_line(
            &format!("Quality: {} (lower is higher quality)", settings.quality),
            [left, y + canvas.r(0.8)],
            (right - left - button * 2.0 - pad).max(0.0),
            BODY,
            TEXT,
        )?;
        for (index, (label, direction)) in [("-", -1), ("+", 1)].into_iter().enumerate() {
            let x = right - button * (2 - index) as f32;
            control(
                canvas,
                view,
                [x, y, x + button - canvas.r(0.4), y + canvas.r(4.0)],
                label,
                Action::AdjustExportQuality(direction),
                enabled,
                Variant::Secondary,
            )?;
        }
        y += canvas.r(5.6);
        let workflow = &view.studio.workflow;
        let path = workflow.destination.as_ref().map_or_else(
            || "Studio exports folder (automatic filenames)".to_owned(),
            |path| path.display().to_string(),
        );
        canvas.text_line(
            "Destination folder or MP4 file",
            [left, y],
            right - left,
            BODY,
            TEXT,
        )?;
        y += canvas.r(3.4);
        widgets::text_area(
            canvas,
            view,
            [left, y, right, y + canvas.r(6.0)],
            workflow.destination_draft.as_deref().unwrap_or(&path),
            "Enter an absolute destination path",
            view.field == Some(launcher::menu::MenuField::StudioDestination),
            MenuAction::Studio(Action::EditExportDestination),
        )?;
        y += canvas.r(7.0);
        if workflow.destination_draft.is_some() {
            let middle = (left + right) * 0.5;
            control(
                canvas,
                view,
                [left, y, middle - pad * 0.5, y + canvas.r(4.0)],
                "Apply destination",
                Action::ApplyExportDestination,
                enabled,
                Variant::Primary,
            )?;
            control(
                canvas,
                view,
                [middle + pad * 0.5, y, right, y + canvas.r(4.0)],
                "Cancel",
                Action::CancelExportDestination,
                true,
                Variant::Secondary,
            )?;
            y += canvas.r(5.0);
        }
        y += canvas.text("Existing files are preserved. Export renders every output frame at the chosen FPS; this cannot recreate missing recorded motion. For slow motion, use a clip speed such as 0.25x with a high FPS source.", [left, y], right - left, CAPTION, TEXT_DIMMER, false)? + canvas.r(2.0);
        row(
            canvas,
            view,
            [left, y, right, bottom],
            if view.studio.export.phase.busy() {
                "Export in progress"
            } else {
                "Export video"
            },
            Action::Export,
            enabled
                && !view.studio.export.phase.busy()
                && project.duration() > recording_studio::MediaTime::ZERO,
        )?;
        y += canvas.r(5.0);
    } else {
        y += canvas.text(
            "Open or create a project to configure output size, FPS, quality and destination.",
            [left, y],
            right - left,
            BODY,
            TEXT_DIMMER,
            false,
        )?;
    }
    let content = y - top + scroll.offset + pad;
    canvas.end_scroll(scroll, content)?;
    Ok(())
}

/// Native wide controls wrap the setting's value in the same focusable hit target.
fn row(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    label: &str,
    action: Action,
    enabled: bool,
) -> Result<(), PaintError> {
    control(
        canvas,
        view,
        [area[0], area[1], area[2], area[1] + canvas.r(4.0)],
        label,
        action,
        enabled,
        Variant::Secondary,
    )
}
