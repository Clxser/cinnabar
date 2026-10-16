//! Export progress stays available across the library and editor without locking the input thread.

use super::*;
use recording_studio::export::ExportPhase;

/// Progress counts admitted output frames; final encoding/mixing remain separately labelled stages.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    let status = &view.studio.export;
    if status.phase == ExportPhase::Idle {
        return Ok(area[1]);
    }
    let pad = canvas.r(1.0);
    let height = canvas.r(if status.notice.is_some() { 12.0 } else { 8.8 });
    let top = area[1];
    canvas.fill([area[0], top, area[2], top + height], NEUTRAL80.fill)?;
    let title = match status.phase {
        ExportPhase::Idle => "",
        ExportPhase::Preparing => "Preparing export...",
        ExportPhase::Rendering => "Rendering video...",
        ExportPhase::FinishingVideo => "Finishing video encoding...",
        ExportPhase::MixingAudio => "Mixing audio...",
        ExportPhase::Muxing => "Adding audio to video...",
        ExportPhase::Publishing => "Publishing finished video...",
        ExportPhase::Complete => "Export complete",
        ExportPhase::Cancelled => "Export cancelled",
        ExportPhase::Failed => "Export failed",
    };
    let button = canvas.r(10.4);
    let right = area[2]
        - pad
        - if status.phase.busy() {
            button + pad
        } else {
            0.0
        };
    canvas.text_line(
        title,
        [area[0] + pad, top + pad],
        right - area[0] - pad,
        BODY,
        TEXT,
    )?;
    let detail = status
        .error
        .as_ref()
        .cloned()
        .or_else(|| status.path.as_ref().map(|path| path.display().to_string()))
        .unwrap_or_else(|| {
            if status.total_frames > 0 {
                format!("{} / {} frames", status.frames, status.total_frames)
            } else {
                "Exporting a snapshot of your current edit.".into()
            }
        });
    canvas.text_line(
        &detail,
        [area[0] + pad, top + canvas.r(3.8)],
        right - area[0] - pad,
        CAPTION,
        TEXT_DIMMER,
    )?;
    if let Some(notice) = &status.notice {
        canvas.text_line(
            notice,
            [area[0] + pad, top + canvas.r(6.6)],
            right - area[0] - pad,
            CAPTION,
            TEXT_DIMMER,
        )?;
    }
    if status.phase.busy() && status.phase != ExportPhase::Publishing {
        control(
            canvas,
            view,
            [
                area[2] - button - pad,
                top + pad,
                area[2] - pad,
                top + canvas.r(5.2),
            ],
            "Cancel",
            Action::CancelExport,
            true,
            Variant::Neutral,
        )?;
    }
    if status.total_frames > 0 {
        let fraction = status.frames.min(status.total_frames) as f32 / status.total_frames as f32;
        canvas.fill(
            [
                area[0] + pad,
                top + height - canvas.r(1.0),
                area[2] - pad,
                top + height - canvas.r(0.6),
            ],
            [50, 55, 60, 255],
        )?;
        canvas.fill(
            [
                area[0] + pad,
                top + height - canvas.r(1.0),
                area[0] + pad + (area[2] - area[0] - pad * 2.0) * fraction,
                top + height - canvas.r(0.6),
            ],
            [93, 154, 58, 255],
        )?;
    }
    Ok(top + height + pad)
}
