//! Compact saved entry controls keep cut, duration and easing beside the selected rig.

use super::*;
use launcher::menu::studio::CameraBlendAction as B;

/// Metadata can be prepared before adjoining shots; runtime eligibility never invents missing source data.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    shot: &CameraShot,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    let enabled = !view
        .studio
        .workspace
        .timeline
        .locked
        .contains(&recording_studio::tracks::Track::Camera);
    let width = (right - left).max(0.0);
    if width < 1.0 {
        return Ok(y);
    }
    let button = canvas.r(3.6).min(width * 0.2);
    let gap = canvas.r(0.3).min(button * 0.2);
    let toggle_right = if shot.blend_in.is_some() {
        (right - button * 2.0).max(left)
    } else {
        right
    };
    control(
        canvas,
        view,
        [left, y, toggle_right, y + canvas.r(3.8)],
        &shot.blend_in.map_or_else(
            || "Entry: Cut".into(),
            |blend| format!("Blend in: {:.3}s", blend.duration.seconds()),
        ),
        Action::CameraBlend(B::Toggle),
        enabled,
        Variant::Secondary,
    )?;
    if let Some(blend) = shot.blend_in {
        for (index, (label, direction)) in [("-", -1), ("+", 1)].into_iter().enumerate() {
            let x = right - button * (2 - index) as f32;
            control(
                canvas,
                view,
                [
                    x.max(left),
                    y,
                    (x + button - gap).max(left),
                    y + canvas.r(3.8),
                ],
                label,
                Action::CameraBlend(B::Duration(direction)),
                enabled,
                Variant::Secondary,
            )?;
        }
        y += canvas.r(4.6);
        control(
            canvas,
            view,
            [left, y, right, y + canvas.r(3.8)],
            &format!("Blend easing: {}", blend.easing.label()),
            Action::CameraBlend(B::CycleEasing),
            enabled,
            Variant::Secondary,
        )?;
    }
    y += canvas.r(4.8);
    Ok(y + canvas.r(0.8))
}

#[cfg(test)]
mod tests;
