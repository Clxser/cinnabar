//! Compact capture controls retain stop, settings and current capture health across Studio views.

use super::*;
use recording_studio::capture::CapturePhase;

/// Idle capture takes one row; only an actionable capture notice consumes additional space.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    let pad = canvas.r(0.8);
    let top = area[1];
    let height = canvas.r(4.0);
    let status = &view.studio.capture;
    let phase = status.phase;
    let label = match phase {
        CapturePhase::Idle | CapturePhase::Complete | CapturePhase::Failed => {
            if view.over_world {
                "Start recording"
            } else {
                "Record next world"
            }
        }
        CapturePhase::Armed => "Cancel recording",
        CapturePhase::Starting => "Cancel startup",
        CapturePhase::Recording => "Stop recording",
        CapturePhase::Finishing => "Finishing...",
    };
    let left_width = canvas.r(21.0).min((area[2] - area[0]) * 0.42);
    control(
        canvas,
        view,
        [area[0], top, area[0] + left_width, top + height],
        label,
        Action::ToggleRecording,
        phase != CapturePhase::Finishing,
        if phase == CapturePhase::Recording {
            Variant::Secondary
        } else {
            Variant::Primary
        },
    )?;
    let settings_width = canvas.r(13.0).min((area[2] - area[0]) * 0.3);
    let settings_left = area[2] - settings_width;
    control(
        canvas,
        view,
        [settings_left, top, area[2], top + height],
        if view.studio.workflow.show_settings {
            "Workspace"
        } else {
            "Settings"
        },
        Action::ToggleSettings,
        true,
        Variant::Secondary,
    )?;
    let mut text_left = area[0] + left_width + pad;
    if phase == CapturePhase::Recording && settings_left - text_left > canvas.r(20.0) {
        let width = canvas.r(12.0);
        control(
            canvas,
            view,
            [text_left, top, text_left + width, top + height],
            "Bookmark",
            Action::Bookmark,
            true,
            Variant::Neutral,
        )?;
        text_left += width + pad;
    }
    let detail = match phase {
        CapturePhase::Idle | CapturePhase::Complete => format!(
            "{}  /  {} FPS",
            view.studio.workflow.capture_settings.size.label(),
            view.studio.workflow.capture_settings.fps
        ),
        CapturePhase::Armed => "Starts when you enter a world".into(),
        CapturePhase::Starting => "Preparing recording...".into(),
        CapturePhase::Recording | CapturePhase::Finishing => {
            let seconds = status.elapsed.0 / recording_studio::time::MICROS_PER_SECOND;
            let actual = if status.elapsed.0 == 0 {
                0.0
            } else {
                status.captured_frames as f64 / status.elapsed.seconds()
            };
            format!(
                "{:02}:{:02}  /  {actual:.0} of {} FPS  /  {} repeated slots",
                seconds / 60,
                seconds % 60,
                status.fps,
                status.missed_frames
            )
        }
        CapturePhase::Failed => "Recording stopped; saved media retained".into(),
    };
    if settings_left - text_left > canvas.r(10.0) {
        canvas.text_line(
            &detail,
            [text_left + pad, top + canvas.r(1.0)],
            settings_left - text_left - pad * 2.0,
            CAPTION,
            TEXT_DIMMER,
        )?;
    }
    let mut bottom = top + height + pad;
    if phase == CapturePhase::Recording {
        let health = &status.health;
        let health_line = format!(
            "Sample CPU {:.2} ms / actors {:.2} ms  |  GPU transfers {}  |  encoder queue {}  |  stalls {}",
            health.sample_micros as f64 / 1000.0,
            health.actor_prepare_micros as f64 / 1000.0,
            health.pending_readbacks,
            health.encoder_frames,
            health.encoder_stalls.saturating_add(health.readback_stalls),
        );
        canvas.text_line(
            &health_line,
            [area[0] + pad, bottom],
            (area[2] - area[0] - pad * 2.0).max(0.0),
            CAPTION,
            TEXT_DIMMER,
        )?;
        bottom += canvas.r(3.0);
    }
    if let Some(message) = &status.message {
        let h = canvas.text(
            message,
            [area[0] + pad, bottom],
            area[2] - area[0] - pad * 2.0,
            CAPTION,
            TEXT_DIMMER,
            false,
        )?;
        bottom += h + pad;
    }
    Ok(bottom)
}
