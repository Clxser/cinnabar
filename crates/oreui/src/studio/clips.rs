//! Clip inspector controls expose real saved source framing, layers and transitions.

use super::*;
use recording_studio::{
    model::{Clip, FramingFit, TransitionKind},
    timeline::FramingParameter,
};

mod timing;

/// Crop, rotation and Fit/Fill keep their own units while sharing ordinary reversible framing commands.
pub(super) fn framing(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    clip: &Clip,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    canvas.text_line("FRAMING", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(4.0);
    let button = canvas.r(3.6);
    for parameter in FramingParameter::ALL.iter().copied() {
        if parameter == FramingParameter::Fit {
            let count = FramingFit::ALL.len() as f32;
            let width = (right - left - canvas.r(0.8) * (count - 1.0)) / count;
            for (index, &mode) in FramingFit::ALL.iter().enumerate() {
                let direction = match mode {
                    FramingFit::Fit => -1,
                    FramingFit::Fill => 1,
                };
                let x = left + index as f32 * (width + canvas.r(0.8));
                control(
                    canvas,
                    view,
                    [x, y, x + width, y + canvas.r(4.0)],
                    mode.label(),
                    Action::AdjustFraming {
                        parameter,
                        direction,
                    },
                    clip.framing.fit != mode,
                    if clip.framing.fit == mode {
                        Variant::Primary
                    } else {
                        Variant::Secondary
                    },
                )?;
            }
            y += canvas.r(4.8);
            continue;
        }
        canvas.text_line(
            &format!(
                "{} {}",
                parameter.label(),
                parameter.formatted(clip.framing)
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
    Ok(y)
}

/// Timing and layer order are the selected clip's default properties.
pub(super) fn general(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    clip: &Clip,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    y = timing::draw(canvas, view, [left, y, right, area[3]], clip)?;
    let native = view
        .studio
        .project
        .as_ref()
        .and_then(|project| {
            project
                .sources
                .iter()
                .find(|source| source.id == clip.source)
        })
        .and_then(recording_studio::capture::native_replay_directory)
        .is_some();
    if native {
        control(
            canvas,
            view,
            [left, y, right, y + canvas.r(4.0)],
            "Use 3D replay",
            Action::UseNativeReplay,
            true,
            Variant::Primary,
        )?;
        y += canvas.r(5.2);
    }
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
    Ok(y)
}

/// Cut controls stay together, with an explicit reason when a neighbouring clip is required.
pub(super) fn cut(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    clip: &Clip,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
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
