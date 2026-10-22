//! Compact viewer monitoring controls never dispatch saved clip-speed or export-range edits.

use super::*;
use launcher::menu::studio::monitoring::MonitoringAction as Monitor;
use recording_studio::playback::monitoring::{JUMP_SECONDS, seek_text};

const ROW_HEIGHT: f32 = 3.5;
const ROW_GAP: f32 = 0.35;
const SINGLE_ROW_WIDTH: f32 = 90.0;

/// The parent reserves the actual control rows, including its own outer padding separately.
pub(super) fn required_height(canvas: &Canvas<'_>, width: f32) -> f32 {
    canvas.r(if width >= canvas.r(SINGLE_ROW_WIDTH) {
        ROW_HEIGHT
    } else {
        ROW_HEIGHT * 2.0 + ROW_GAP
    })
}

/// Validated rows keep their hit targets inside the viewer even when the editor is short or narrow.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), PaintError> {
    let width = area[2] - area[0];
    let height = area[3] - area[1];
    if width <= canvas.r(12.0) || height < canvas.r(3.0) {
        return Ok(());
    }
    let single_row = width >= canvas.r(SINGLE_ROW_WIDTH);
    let gap = canvas.r(ROW_GAP);
    let expanded = single_row || height + 0.1 >= required_height(canvas, width);
    let row_height = if expanded && !single_row {
        (height - gap) * 0.5
    } else {
        height.min(canvas.r(ROW_HEIGHT))
    };
    let enabled = view
        .studio
        .project
        .as_ref()
        .is_some_and(|p| p.duration() > MediaTime::ZERO);
    let jump_back = format!("-{JUMP_SECONDS}s");
    let jump_forward = format!("+{JUMP_SECONDS}s");
    let commands = [
        ("|<", Action::Monitoring(Monitor::Start), false),
        (
            jump_back.as_str(),
            Action::Monitoring(Monitor::Jump(-JUMP_SECONDS)),
            false,
        ),
        ("<", Action::Step(-1), false),
        (
            if view.studio.playing { "Pause" } else { "Play" },
            Action::TogglePlayback,
            true,
        ),
        (">", Action::Step(1), false),
        (
            jump_forward.as_str(),
            Action::Monitoring(Monitor::Jump(JUMP_SECONDS)),
            false,
        ),
        (">|", Action::Monitoring(Monitor::End), false),
    ];
    let navigation_width = if single_row { canvas.r(36.0) } else { width };
    let button = (navigation_width - gap * (commands.len() - 1) as f32) / commands.len() as f32;
    for (index, (label, action, primary)) in commands.into_iter().enumerate() {
        let x = area[0] + index as f32 * (button + gap);
        control(
            canvas,
            view,
            [x, area[1], x + button, area[1] + row_height],
            label,
            action,
            enabled,
            if primary {
                Variant::Primary
            } else {
                Variant::Secondary
            },
        )?;
    }
    if !expanded {
        return Ok(());
    }
    let y = if single_row {
        area[1]
    } else {
        area[1] + row_height + gap
    };
    let unit = (width / canvas.r(55.0)).min(1.0);
    let small = canvas.r(3.0) * unit;
    let rate = canvas.r(13.0) * unit;
    let loop_width = canvas.r(6.5) * unit;
    let mut x = area[0]
        + if single_row {
            navigation_width + canvas.r(1.2)
        } else {
            0.0
        };
    let speed_label = format!("Monitor {}", view.studio.workspace.monitoring.speed.label());
    for (label, action, w, active) in [
        ("-", Monitor::StepSpeed(-1), small, false),
        (
            speed_label.as_str(),
            Monitor::Speed(Default::default()),
            rate,
            false,
        ),
        ("+", Monitor::StepSpeed(1), small, false),
        (
            "Loop",
            Monitor::ToggleLoop,
            loop_width,
            view.studio.workspace.monitoring.looping,
        ),
    ] {
        control(
            canvas,
            view,
            [x, y, x + w, area[3]],
            label,
            Action::Monitoring(action),
            enabled,
            if active {
                Variant::Primary
            } else {
                Variant::Secondary
            },
        )?;
        x += w + gap;
    }
    let go_width = canvas.r(3.8) * unit;
    let field_right = area[2] - go_width - gap;
    if field_right <= x + canvas.r(5.0) {
        return Ok(());
    }
    let monitor = &view.studio.workspace.monitoring;
    let value = seek_text(view.studio.position);
    widgets::text_field(
        canvas,
        view,
        [x, y, field_right, area[3]],
        monitor.seek_draft.as_deref().unwrap_or(&value),
        "Seek seconds / timecode",
        monitor.seek_draft.is_some() && view.field == Some(launcher::menu::MenuField::StudioSeek),
        Some(MenuAction::Studio(Action::Monitoring(Monitor::EditSeek))),
    )?;
    control(
        canvas,
        view,
        [field_right + gap, y, area[2], area[3]],
        "Go",
        Action::Monitoring(Monitor::ApplySeek),
        enabled && monitor.seek_draft.is_some(),
        Variant::Secondary,
    )
}
