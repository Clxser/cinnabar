//! Clip inspector controls expose real saved source framing, layers and transitions.

use super::*;
use recording_studio::{
    model::{Clip, TransitionKind},
    timeline::FramingParameter,
};

mod timing;

/// Keeps percentage controls legible on narrow inspectors and derives labels from the domain owner.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    clip: &Clip,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    y = timing::draw(canvas, view, [left, y, right, area[3]], clip)?;
    canvas.text_line("FRAMING", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(4.0);
    let button = canvas.r(3.6);
    for parameter in FramingParameter::ALL.iter().copied() {
        canvas.text_line(
            &format!(
                "{} {:.1}%",
                parameter.label(),
                parameter.value(clip.framing) * 100.0
            ),
            [left, y + canvas.r(0.8)],
            (right - left - button * 2.0 - canvas.r(1.2)).max(0.0),
            CAPTION,
            TEXT_DIMMER,
        )?;
        for (index, (label, direction)) in [("-", -1), ("+", 1)].into_iter().enumerate() {
            let x = right - (2 - index) as f32 * (button + canvas.r(0.4));
            control(
                canvas,
                view,
                [x, y, x + button, y + button],
                label,
                Action::AdjustFraming {
                    parameter,
                    direction,
                },
                true,
                Variant::Secondary,
            )?;
        }
        y += canvas.r(4.4);
    }
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(4.0)],
        "Reset framing",
        Action::ResetFraming,
        true,
        Variant::Secondary,
    )?;
    y += canvas.r(5.6);
    canvas.text_line(
        &format!("Visual layer {}", clip.track + 1),
        [left, y],
        right - left,
        BODY,
        TEXT,
    )?;
    y += canvas.r(3.6);
    let width = (right - left - canvas.r(0.8)) * 0.5;
    for (index, (label, direction)) in [("Lower", -1), ("Higher", 1)].into_iter().enumerate() {
        let x = left + index as f32 * (width + canvas.r(0.8));
        control(
            canvas,
            view,
            [x, y, x + width, y + canvas.r(4.0)],
            label,
            Action::MoveClipLayer(direction),
            if direction < 0 {
                clip.track > 0
            } else {
                clip.track + 1 < recording_studio::model::MAX_VIDEO_TRACKS
            },
            Variant::Secondary,
        )?;
    }
    y += canvas.r(6.0);
    let project = view
        .studio
        .project
        .as_ref()
        .expect("clip inspector has project");
    let next = clip.span().ok().is_some_and(|span| {
        project
            .clips
            .iter()
            .any(|next| next.enabled && next.track == clip.track && next.at == span.end)
    });
    for (label, kind) in [
        ("Crossfade to next", TransitionKind::Crossfade),
        ("Dip to black", TransitionKind::DipToBlack),
    ] {
        control(
            canvas,
            view,
            [left, y, right, y + canvas.r(4.0)],
            label,
            Action::TransitionToNext(kind),
            next,
            Variant::Secondary,
        )?;
        y += canvas.r(4.8);
    }
    if !next {
        y += canvas.text(
            "Transitions need adjacent clips on this layer.",
            [left, y],
            right - left,
            CAPTION,
            TEXT_DIMMER,
            false,
        )? + canvas.r(0.8);
    }
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(4.0)],
        "Remove transition",
        Action::RemoveClipTransition,
        project
            .transitions
            .iter()
            .any(|transition| transition.from == clip.id),
        Variant::Secondary,
    )?;
    Ok(y + canvas.r(6.0))
}
