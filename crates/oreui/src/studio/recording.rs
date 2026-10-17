//! Native live-capture controls remain available while browsing or editing a project.

use super::*;
use recording_studio::capture::CapturePhase;

/// Keeps cancellation and progress visible without replacing a creator's current document.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    let pad = canvas.r(1.2);
    let top = area[1];
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
    let button_right = (area[0] + canvas.r(23.0)).min(area[2]);
    control(
        canvas,
        view,
        [area[0], top, button_right, top + canvas.r(4.8)],
        label,
        Action::ToggleRecording,
        phase != CapturePhase::Finishing,
        if phase == CapturePhase::Recording {
            Variant::Secondary
        } else {
            Variant::Primary
        },
    )?;
    let bookmark_left = button_right + pad;
    if area[2] - bookmark_left > canvas.r(17.0) {
        control(
            canvas,
            view,
            [
                bookmark_left,
                top,
                bookmark_left + canvas.r(16.0),
                top + canvas.r(4.8),
            ],
            "Bookmark",
            Action::Bookmark,
            phase == CapturePhase::Recording,
            Variant::Neutral,
        )?;
    }
    let settings_width = canvas.r(20.0);
    let settings_left = (area[2] - settings_width).max(bookmark_left + canvas.r(17.2));
    let stacked = settings_left > area[2] - settings_width;
    let settings_top = if stacked { top + canvas.r(5.6) } else { top };
    let settings_left = if stacked { area[0] } else { settings_left };
    control(
        canvas,
        view,
        [
            settings_left,
            settings_top,
            area[2],
            settings_top + canvas.r(4.8),
        ],
        if view.studio.workflow.show_settings {
            "Back to workspace"
        } else {
            "Recording / export"
        },
        Action::ToggleSettings,
        true,
        Variant::Secondary,
    )?;
    let message_top = settings_top + canvas.r(5.6);
    let seconds = status.elapsed.0 / recording_studio::time::MICROS_PER_SECOND;
    let mut message = status.message.clone().unwrap_or_else(|| match phase {
        CapturePhase::Idle => "Records your current view and game audio.".into(),
        CapturePhase::Armed => "Ready: recording starts when you enter a world.".into(),
        CapturePhase::Starting => "Preparing capture and video encoding...".into(),
        CapturePhase::Recording => format!("Recording {:02}:{:02}", seconds / 60, seconds % 60),
        CapturePhase::Finishing => "Saving video, game audio and event markers...".into(),
        CapturePhase::Complete => "Recording saved in your studio.".into(),
        CapturePhase::Failed => "Recording stopped. Your saved media has been retained.".into(),
    });
    if matches!(
        phase,
        CapturePhase::Recording | CapturePhase::Finishing | CapturePhase::Complete
    ) {
        let actual = if status.elapsed.0 == 0 {
            0.0
        } else {
            status.captured_frames as f64 / status.elapsed.seconds()
        };
        message.push_str(&format!(
            "  |  target {} FPS, {actual:.1} captured FPS, {} missed/repeated slots",
            status.fps, status.missed_frames
        ));
    }
    let message_height = canvas.text(
        &message,
        [area[0], message_top],
        area[2] - area[0],
        CAPTION,
        TEXT_DIMMER,
        false,
    )?;
    Ok((message_top + message_height + pad).max(message_top + canvas.r(3.6)))
}
