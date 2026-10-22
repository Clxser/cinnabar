//! Compact viewer monitoring controls never dispatch saved clip-speed or export-range edits.

use super::*;
use launcher::menu::studio::monitoring::MonitoringAction as Monitor;
use recording_studio::playback::monitoring::{JUMP_SECONDS, seek_text};

const ROW_HEIGHT: f32 = 3.5;
const ROW_GAP: f32 = 0.35;
const NAVIGATION_COUNT: usize = 7;

/// Both measurement and paint read the same complete navigation command row.
fn commands<'a>(
    view: &MenuView,
    back: &'a str,
    forward: &'a str,
) -> [(&'a str, Action, bool); NAVIGATION_COUNT] {
    [
        ("|<", Action::Monitoring(Monitor::Start), false),
        (
            back,
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
            forward,
            Action::Monitoring(Monitor::Jump(JUMP_SECONDS)),
            false,
        ),
        (">|", Action::Monitoring(Monitor::End), false),
    ]
}

struct Geometry {
    single: bool,
    navigation: f32,
    buttons: [f32; NAVIGATION_COUNT],
    small: f32,
    rate: f32,
    loop_width: f32,
    go: f32,
    height: f32,
}

/// Native label widths determine the actual one-row threshold, including a readable seek value.
fn geometry(canvas: &mut Canvas<'_>, view: &MenuView, width: f32) -> Result<Geometry, PaintError> {
    let gap = canvas.r(ROW_GAP);
    let mut buttons = [canvas.r(3.2); NAVIGATION_COUNT];
    for (index, (label, _, primary)) in commands(
        view,
        &format!("-{JUMP_SECONDS}s"),
        &format!("+{JUMP_SECONDS}s"),
    )
    .into_iter()
    .enumerate()
    {
        buttons[index] = buttons[index].max(
            canvas.measure(
                if primary { "Pause" } else { label },
                crate::theme::SECONDARY_BUTTON,
            )? + canvas.r(1.4),
        );
    }
    let navigation = buttons.iter().sum::<f32>() + gap * (NAVIGATION_COUNT - 1) as f32;
    let small = canvas.r(3.0);
    let rate = canvas.measure(
        &format!("Monitor {}", view.studio.workspace.monitoring.speed.label()),
        crate::theme::SECONDARY_BUTTON,
    )? + canvas.r(1.4);
    let loop_width = canvas.measure("Loop", crate::theme::SECONDARY_BUTTON)? + canvas.r(1.4);
    let go = canvas.measure("Go", crate::theme::SECONDARY_BUTTON)? + canvas.r(1.4);
    let seek = canvas.measure(&seek_text(view.studio.position), BODY)? + canvas.r(2.0);
    let minimum =
        navigation + canvas.r(1.2) + small * 2.0 + rate + loop_width + go + seek + gap * 5.0;
    let single = width + 0.1 >= minimum;
    let unit = if single {
        1.0
    } else {
        (width / canvas.r(55.0)).min(1.0)
    };
    Ok(Geometry {
        single,
        navigation,
        buttons,
        small: small * unit,
        rate: rate * unit,
        loop_width: loop_width * unit,
        go: go * unit,
        height: canvas.r(if single {
            ROW_HEIGHT
        } else {
            ROW_HEIGHT * 2.0 + ROW_GAP
        }),
    })
}

/// The parent reserves the actual control rows, including its own outer padding separately.
pub(super) fn required_height(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    width: f32,
) -> Result<f32, PaintError> {
    Ok(geometry(canvas, view, width)?.height)
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
    let layout = geometry(canvas, view, width)?;
    let single_row = layout.single;
    let gap = canvas.r(ROW_GAP);
    let expanded = single_row || height + 0.1 >= layout.height;
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
    let commands = commands(view, &jump_back, &jump_forward);
    let navigation_width = if single_row { layout.navigation } else { width };
    let button = (navigation_width - gap * (commands.len() - 1) as f32) / commands.len() as f32;
    let mut x = area[0];
    for (index, (label, action, primary)) in commands.into_iter().enumerate() {
        let button = if single_row {
            layout.buttons[index]
        } else {
            button
        };
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
        x += button + gap;
    }
    if !expanded {
        return Ok(());
    }
    let y = if single_row {
        area[1]
    } else {
        area[1] + row_height + gap
    };
    let small = layout.small;
    let rate = layout.rate;
    let loop_width = layout.loop_width;
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
    let go_width = layout.go;
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
