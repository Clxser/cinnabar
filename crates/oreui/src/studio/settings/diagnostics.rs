//! Setup checks report proven encoder availability rather than inferring it from GPU vendor names.

use super::*;

/// Explicit checks run on a worker; displaying their last result performs no file or process work.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    let diagnostics = &view.studio.workflow.diagnostics;
    canvas.text_line("Recording setup", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(4.4);
    y += canvas.text("Replay-only recording needs no video encoder. Video imports and exports use FFmpeg and FFprobe. This check encodes a frame to verify actual CPU and NVIDIA support.",[left,y],right-left,CAPTION,TEXT_DIMMER,false)?+canvas.r(2.0);
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(4.0)],
        if diagnostics.checking {
            "Checking dependencies..."
        } else {
            "Check dependencies"
        },
        Action::CheckDependencies,
        !diagnostics.checking
            && !view.studio.capture.phase.busy()
            && !view.studio.export.phase.busy(),
        Variant::Primary,
    )?;
    y += canvas.r(5.0);
    if let Some(notice) = &view.studio.preview.notice {
        canvas.text_line("Replay details", [left, y], right - left, BODY, TEXT)?;
        y += canvas.r(3.4);
        y += canvas.text(notice, [left, y], right - left, CAPTION, TEXT_DIMMER, false)?
            + canvas.r(2.0);
    }
    if let Some(error) = &diagnostics.error {
        y += canvas.text(error, [left, y], right - left, BODY, TEXT, false)? + canvas.r(2.0);
    }
    if let Some(report) = &diagnostics.report {
        canvas.text_line("FFmpeg found", [left, y], right - left, BODY, TEXT)?;
        y += canvas.r(3.2);
        y += canvas.text(
            &report.version,
            [left, y],
            right - left,
            CAPTION,
            TEXT_DIMMER,
            false,
        )? + canvas.r(1.2);
        y += canvas.text(
            &report.executable.display().to_string(),
            [left, y],
            right - left,
            CAPTION,
            TEXT_DIMMER,
            false,
        )? + canvas.r(2.0);
        canvas.text_line(
            if report.ffprobe.is_ok() {
                "FFprobe inspection: Ready"
            } else {
                "FFprobe inspection: Unavailable"
            },
            [left, y],
            right - left,
            BODY,
            TEXT,
        )?;
        y += canvas.r(3.2);
        if let Err(error) = &report.ffprobe {
            y += canvas.text(error, [left, y], right - left, CAPTION, TEXT_DIMMER, false)?
                + canvas.r(2.0);
            y += canvas.text("Export verification needs FFprobe. Install it beside FFmpeg or add it to PATH, then check dependencies again.", [left, y], right - left, CAPTION, TEXT_DIMMER, false)? + canvas.r(2.0);
        }
        canvas.text_line(
            &format!("Checked format: {}", codec_label(report.codec)),
            [left, y],
            right - left,
            BODY,
            TEXT,
        )?;
        y += canvas.r(3.8);
        for (name, probe) in [
            ("CPU encoding", &report.software),
            ("NVIDIA encoding", &report.nvidia),
        ] {
            canvas.text_line(
                &format!(
                    "{name}: {}",
                    if probe.available {
                        "Ready"
                    } else {
                        "Unavailable"
                    }
                ),
                [left, y],
                right - left,
                BODY,
                TEXT,
            )?;
            y += canvas.r(3.0);
            if let Some(error) = &probe.error {
                y += canvas.text(error, [left, y], right - left, CAPTION, TEXT_DIMMER, false)?
                    + canvas.r(1.4);
            }
        }
        if report.cpu_fallback {
            y+=canvas.text("NVIDIA encoding is unavailable; CPU encoding is available as a fallback. Live video encoding may reduce game FPS. Use replay-only recording for lighter capture.",[left,y],right-left,CAPTION,TEXT_DIMMER,false)?+canvas.r(2.0);
        }
        control(
            canvas,
            view,
            [left, y, right, y + canvas.r(4.0)],
            "Use CPU for this project's export",
            Action::UseCpuEncoder,
            report.software.available
                && view
                    .studio
                    .project
                    .as_ref()
                    .is_some_and(|project| project.export.codec == report.codec)
                && !view.studio.export.phase.busy(),
            Variant::Secondary,
        )?;
        y += canvas.r(5.0);
    }
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(4.0)],
        "Project recovery",
        Action::ShowRecovery,
        !view.studio.workflow.recovery.busy,
        Variant::Neutral,
    )?;
    Ok(y + canvas.r(5.0))
}
