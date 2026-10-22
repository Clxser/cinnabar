//! Export progress stays available without replacing the editor workspace.

use super::*;
use recording_studio::export::ExportPhase;

/// Shows the current export stage with bounded detail and a persistent cancellation action.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    let status = &view.studio.export;
    if status.phase == ExportPhase::Idle || area[2] <= area[0] {
        return Ok(area[1]);
    }
    let pad = canvas.r(0.8);
    let top = area[1];
    let row = canvas.r(4.6);
    let height = row
        + if status.notice.is_some() {
            canvas.r(2.5)
        } else {
            0.0
        };
    canvas.fill([area[0], top, area[2], top + height], NEUTRAL80.fill)?;
    let title = match status.phase {
        ExportPhase::Idle => "",
        ExportPhase::Preparing => "Preparing export...",
        ExportPhase::Rendering => "Rendering video...",
        ExportPhase::FinishingVideo => "Finishing video encoding...",
        ExportPhase::MixingAudio => "Mixing audio...",
        ExportPhase::Muxing => "Adding audio...",
        ExportPhase::Verifying => "Verifying video and audio...",
        ExportPhase::Publishing => "Publishing video...",
        ExportPhase::Complete => "Export complete",
        ExportPhase::Cancelled => "Export cancelled",
        ExportPhase::Failed => "Export failed",
    };
    let can_cancel = status.phase.busy() && status.phase != ExportPhase::Publishing;
    let button = canvas.r(10.0);
    let right = area[2] - pad - if can_cancel { button + pad } else { 0.0 };
    let content_width = (right - area[0] - pad).max(0.0);
    let detail = status
        .error
        .as_ref()
        .cloned()
        .or_else(|| status.path.as_ref().map(|path| path.display().to_string()))
        .unwrap_or_else(|| format!("{} / {} frames", status.frames, status.total_frames));
    let title_width = canvas.r(29.0).min(content_width);
    canvas.text_line(title, [area[0] + pad, top + pad], title_width, BODY, TEXT)?;
    if content_width > title_width + canvas.r(12.0) {
        canvas.text_line(
            &detail,
            [area[0] + pad * 2.0 + title_width, top + canvas.r(1.0)],
            (content_width - title_width - pad).max(0.0),
            CAPTION,
            TEXT_DIMMER,
        )?;
    } else {
        canvas.text_line(
            &detail,
            [area[0] + pad, top + canvas.r(2.6)],
            content_width,
            CAPTION,
            TEXT_DIMMER,
        )?;
    }
    if let Some(notice) = &status.notice {
        canvas.text_line(
            notice,
            [area[0] + pad, top + row],
            content_width,
            CAPTION,
            TEXT_DIMMER,
        )?;
    }
    if can_cancel {
        control(
            canvas,
            view,
            [
                area[2] - button - pad,
                top + pad * 0.5,
                area[2] - pad,
                top + row - pad,
            ],
            "Cancel",
            Action::CancelExport,
            true,
            Variant::Neutral,
        )?;
    }
    if status.total_frames > 0 && content_width > 0.0 {
        let fraction = status.frames.min(status.total_frames) as f32 / status.total_frames as f32;
        let bounds = [
            area[0] + pad,
            top + height - canvas.r(0.4),
            right,
            top + height - canvas.r(0.2),
        ];
        canvas.fill(bounds, [50, 55, 60, 255])?;
        if fraction > 0.0 {
            canvas.fill(
                [
                    bounds[0],
                    bounds[1],
                    bounds[0] + content_width * fraction,
                    bounds[3],
                ],
                [93, 154, 58, 255],
            )?;
        }
    }
    Ok(top + height + pad)
}
