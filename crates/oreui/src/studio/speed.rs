//! Speed controls edit one source clock shared by visual frames, linked audio and generated events.

use super::*;
use recording_studio::{PlaybackRate, model::Clip};

/// The same derived multipliers are available for constant playback and individual ramp keys.
pub(super) fn rates(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    ramp: bool,
    enabled: bool,
) -> Result<f32, PaintError> {
    let [left, y, right, _] = area;
    let gap = canvas.r(0.8);
    let width = (right - left - gap * 2.0) / 3.0;
    for (index, power) in (-2i32..=2).enumerate() {
        let factor = 1u32 << power.unsigned_abs();
        let rate = if power < 0 {
            PlaybackRate {
                numerator: 1,
                denominator: factor,
            }
        } else {
            PlaybackRate {
                numerator: factor,
                denominator: 1,
            }
        };
        let label = format!(
            "{}x",
            f64::from(rate.numerator) / f64::from(rate.denominator)
        );
        let x = left + (index % 3) as f32 * (width + gap);
        let top = y + (index / 3) as f32 * canvas.r(4.8);
        let action = if ramp {
            Action::SetSpeedKey {
                rate,
                ripple: view.studio.workspace.ripple,
            }
        } else {
            Action::SetRate(rate)
        };
        control(
            canvas,
            view,
            [x, top, x + width, top + canvas.r(4.0)],
            &label,
            action,
            enabled,
            Variant::Secondary,
        )?;
    }
    Ok(y + canvas.r(10.4))
}

/// Only a bounded neighborhood of keys is painted; previous/next navigation reaches long ramps.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    clip: &Clip,
) -> Result<f32, PaintError> {
    let [left, mut y, right, bottom] = area;
    canvas.text_line("SPEED RAMP", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(4.0);
    y += canvas.text("Set a multiplier at the playhead. Smooth keys ease the speed between source moments. Ramp audio changes pitch.", [left, y], right - left, CAPTION, TEXT_DIMMER, false)? + canvas.r(1.2);
    let map = clip.time_map().ok();
    let source = map
        .as_ref()
        .and_then(|map| map.source_boundary(view.studio.position).ok());
    let inside = clip
        .span()
        .is_ok_and(|span| view.studio.position >= span.start && view.studio.position <= span.end);
    y = rates(canvas, view, [left, y, right, bottom], true, inside)?;
    let Some(ramp) = &clip.speed_ramp else {
        return Ok(y);
    };
    let selected = view
        .studio
        .speed_key
        .filter(|(id, _)| *id == clip.id)
        .map(|(_, at)| at)
        .or(source);
    let exact = selected.and_then(|at| ramp.key_at(at));
    let ripple = view.studio.workspace.ripple;
    let width = (right - left - canvas.r(0.8)) * 0.5;
    for (index, (label, action)) in [
        ("Prev key", Action::StepSpeedKey(-1)),
        ("Next key", Action::StepSpeedKey(1)),
    ]
    .into_iter()
    .enumerate()
    {
        let x = left + index as f32 * (width + canvas.r(0.8));
        control(
            canvas,
            view,
            [x, y, x + width, y + canvas.r(4.0)],
            label,
            action,
            inside,
            Variant::Secondary,
        )?;
    }
    y += canvas.r(4.8);
    let easing = exact.map_or("Select a key".to_string(), |key| {
        format!("Incoming: {}", key.easing.label())
    });
    for (label, action, enabled) in [
        (
            easing.as_str(),
            Action::CycleSpeedEasing { ripple },
            exact.is_some(),
        ),
        (
            "Delete speed key",
            Action::DeleteSpeedKey { ripple },
            exact.is_some(),
        ),
        ("Clear ramp", Action::ClearSpeedRamp { ripple }, inside),
    ] {
        control(
            canvas,
            view,
            [left, y, right, y + canvas.r(4.0)],
            label,
            action,
            enabled,
            Variant::Secondary,
        )?;
        y += canvas.r(4.8);
    }
    let retained: Vec<_> = ramp
        .keys
        .iter()
        .filter(|key| key.at >= clip.source_range.start && key.at <= clip.source_range.end)
        .collect();
    let center = retained.partition_point(|key| Some(key.at) < selected);
    let start = center
        .saturating_sub(6)
        .min(retained.len().saturating_sub(12));
    for key in retained.iter().skip(start).take(12) {
        let label = format!(
            "{} · {}x",
            time_label(key.at),
            f64::from(key.rate.numerator) / f64::from(key.rate.denominator)
        );
        let active = selected == Some(key.at);
        control(
            canvas,
            view,
            [left, y, right, y + canvas.r(4.0)],
            &label,
            Action::SelectSpeedKey {
                clip: clip.id,
                at: key.at,
            },
            true,
            if active {
                Variant::Primary
            } else {
                Variant::Secondary
            },
        )?;
        y += canvas.r(4.8);
    }
    if retained.len() > 12 {
        y += canvas.text(
            "Use Prev/Next key to reach more keys.",
            [left, y],
            right - left,
            CAPTION,
            TEXT_DIMMER,
            false,
        )?;
    }
    Ok(y + canvas.r(2.0))
}
