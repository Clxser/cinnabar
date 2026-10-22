//! Native curve controls edit the same effects generated from gameplay evidence.

use super::*;
use recording_studio::{
    MediaTime,
    effect::{Easing, EffectClip, EffectKind},
};

/// Available effects and their names come from the domain schema, with no parallel menu registry.
pub(super) fn creation(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    canvas.text_line("LOOKS", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(3.8);
    let creative = &view.studio.workflow.creative;
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(3.6)],
        &format!("Strength: {}%", creative.look_strength),
        Action::Creative(launcher::menu::studio::creative::CreativeAction::CycleLookStrength),
        true,
        Variant::Secondary,
    )?;
    y += canvas.r(4.4);
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(3.6)],
        if creative.look_selected_range {
            "Range: selected clip"
        } else {
            "Range: at playhead"
        },
        Action::Creative(launcher::menu::studio::creative::CreativeAction::ToggleLookRange),
        true,
        Variant::Secondary,
    )?;
    y += canvas.r(4.6);
    let enabled = view
        .studio
        .project
        .as_ref()
        .is_some_and(|project| project.duration() > MediaTime::ZERO);
    let pad = canvas.r(0.8);
    let width = (right - left - pad) * 0.5;
    for (index, look) in recording_studio::effect::preset::Look::ALL
        .iter()
        .copied()
        .enumerate()
    {
        let x = left + (index % 2) as f32 * (width + pad);
        let top = y + (index / 2) as f32 * canvas.r(4.8);
        control(
            canvas,
            view,
            [x, top, x + width, top + canvas.r(4.0)],
            look.label(),
            Action::AddLook(look),
            enabled,
            Variant::Secondary,
        )?;
    }
    y += recording_studio::effect::preset::Look::ALL
        .len()
        .div_ceil(2) as f32
        * canvas.r(4.8)
        + canvas.r(2.0);
    canvas.text_line("ADD EFFECT", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(3.8);
    for (index, kind) in EffectKind::ALL.iter().copied().enumerate() {
        let x = left + (index % 2) as f32 * (width + pad);
        let top = y + (index / 2) as f32 * canvas.r(4.8);
        control(
            canvas,
            view,
            [x, top, x + width, top + canvas.r(4.0)],
            kind.label(),
            Action::AddEffect(kind),
            enabled,
            Variant::Secondary,
        )?;
    }
    Ok(y + EffectKind::ALL.len().div_ceil(2) as f32 * canvas.r(4.8))
}

/// Amount controls key the exact playhead; end-point keys remain available through key navigation.
pub(super) fn inspector(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    effect: &EffectClip,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    canvas.text_line(effect.kind.label(), [left, y], right - left, BODY, TEXT)?;
    y += canvas.r(3.4);
    canvas.text_line(
        if effect.origin.is_some() {
            "From gameplay - editable"
        } else {
            "Manual effect"
        },
        [left, y],
        right - left,
        CAPTION,
        TEXT_DIMMER,
    )?;
    y += canvas.r(3.0);
    canvas.text_line(
        &format!(
            "{} - {}",
            time_label(effect.span.start),
            time_label(effect.span.end)
        ),
        [left, y],
        right - left,
        CAPTION,
        TEXT_DIMMER,
    )?;
    y += canvas.r(3.2);
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(4.0)],
        if effect.enabled {
            "Enabled"
        } else {
            "Disabled"
        },
        Action::ToggleEffect,
        true,
        if effect.enabled {
            Variant::Primary
        } else {
            Variant::Secondary
        },
    )?;
    y += canvas.r(5.2);
    let at = view.studio.position;
    let valid = at >= effect.span.start && at <= effect.span.end;
    let offset = at.saturating_sub(effect.span.start);
    let key = effect.amount.keys.iter().find(|key| key.at == offset);
    let amount = effect.amount.sample(offset);
    let amount_label = if valid {
        format!("Amount at playhead: {amount:.3}")
    } else {
        "Playhead outside effect".into()
    };
    canvas.text_line(&amount_label, [left, y], right - left, CAPTION, TEXT)?;
    y += canvas.r(3.2);
    y = pair(
        canvas,
        view,
        [left, y, right, area[3]],
        [
            ("-", Action::AdjustEffect(-1)),
            ("+", Action::AdjustEffect(1)),
        ],
        valid,
    )?;
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(3.8)],
        if view.studio.camera_fine {
            "Fine adjustment: On"
        } else {
            "Fine adjustment: Off"
        },
        Action::ToggleCameraFine,
        true,
        Variant::Secondary,
    )?;
    y += canvas.r(5.0);
    canvas.text_line(
        &format!("{} keys", effect.amount.keys.len()),
        [left, y],
        right - left,
        CAPTION,
        TEXT_DIMMER,
    )?;
    y += canvas.r(3.0);
    graph(
        canvas,
        [left, y, right, y + canvas.r(7.0)],
        effect,
        valid.then_some(offset),
    )?;
    y += canvas.r(8.2);
    y = pair(
        canvas,
        view,
        [left, y, right, area[3]],
        [
            ("Prev key", Action::StepEffectKey(-1)),
            ("Next key", Action::StepEffectKey(1)),
        ],
        true,
    )?;
    let width = (right - left - canvas.r(0.8)) * 0.5;
    control(
        canvas,
        view,
        [left, y, left + width, y + canvas.r(4.0)],
        "Add key",
        Action::AddEffectKey,
        valid,
        Variant::Secondary,
    )?;
    control(
        canvas,
        view,
        [right - width, y, right, y + canvas.r(4.0)],
        "Delete key",
        Action::DeleteEffectKey,
        valid && key.is_some() && effect.amount.keys.len() > 1,
        Variant::Secondary,
    )?;
    y += canvas.r(5.2);
    let easing = key.map_or(Easing::Smooth, |key| key.easing);
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(4.0)],
        &format!("Incoming easing: {}", easing.label()),
        Action::CycleEffectEasing,
        valid,
        Variant::Secondary,
    )?;
    y += canvas.r(5.2);
    canvas.text_line(
        "Move one output frame",
        [left, y],
        right - left,
        CAPTION,
        TEXT_DIMMER,
    )?;
    y += canvas.r(3.0);
    y = pair(
        canvas,
        view,
        [left, y, right, area[3]],
        [
            ("Earlier", Action::MoveEffect(-1)),
            ("Later", Action::MoveEffect(1)),
        ],
        true,
    )?;
    Ok(y)
}

/// The graph samples actual interpolation, with work bounded independently of key count.
fn graph(
    canvas: &mut Canvas<'_>,
    area: Bounds,
    effect: &EffectClip,
    offset: Option<MediaTime>,
) -> Result<(), PaintError> {
    canvas.fill(area, NEUTRAL90)?;
    let mut low = 0.0_f64;
    let mut high = effect.kind.initial().max(0.01);
    for key in &effect.amount.keys {
        let value = effect.kind.constrain(key.value);
        low = low.min(value);
        high = high.max(value);
    }
    let height = (area[3] - area[1] - canvas.r(0.8)).max(1.0);
    let width = (area[2] - area[0]).max(1.0);
    let mut previous = area[3];
    const SAMPLES: usize = 64;
    for sample in 0..SAMPLES {
        let time = MediaTime(effect.span.duration().0 * sample as u64 / (SAMPLES - 1) as u64);
        let value = effect.kind.constrain(effect.amount.sample(time));
        let y = area[3] - canvas.r(0.4) - ((value - low) / (high - low).max(0.01)) as f32 * height;
        let x = area[0] + sample as f32 / SAMPLES as f32 * width;
        if sample > 0 {
            canvas.fill(
                [
                    x,
                    y.min(previous),
                    (x + width / SAMPLES as f32).min(area[2]),
                    y.max(previous) + 1.0,
                ],
                TEXT_DIMMER,
            )?;
        }
        previous = y;
    }
    if let Some(offset) = offset {
        let x = (area[0] + (offset.0 as f64 / effect.span.duration().0 as f64) as f32 * width)
            .min((area[2] - 1.0).max(area[0]));
        canvas.fill([x, area[1], (x + 1.0).min(area[2]), area[3]], TEXT)?;
    }
    Ok(())
}

/// Paired authoring buttons share sizing, focus and disabled behavior with other native Studio controls.
fn pair(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    actions: [(&str, Action); 2],
    enabled: bool,
) -> Result<f32, PaintError> {
    let width = (area[2] - area[0] - canvas.r(0.8)) * 0.5;
    for (index, (label, action)) in actions.into_iter().enumerate() {
        let x = area[0] + index as f32 * (width + canvas.r(0.8));
        control(
            canvas,
            view,
            [x, area[1], x + width, area[1] + canvas.r(4.0)],
            label,
            action,
            enabled,
            Variant::Secondary,
        )?;
    }
    Ok(area[1] + canvas.r(5.2))
}
