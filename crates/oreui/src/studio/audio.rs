//! Selected audio clips expose real gain, mute and output-time fades through ordinary undoable edits.

use super::*;
use recording_studio::{audio::AudioParameter, model::AudioClip};

/// The parent owns scrolling and dispatch; this panel never changes the linked source clock.
pub(super) fn inspector(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    clip: &AudioClip,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    canvas.text_line("AUDIO MIX", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(4.0);
    if clip.linked_clip.is_some() {
        y += canvas.text(
            "Linked audio: timing follows its video clip.",
            [left, y],
            right - left,
            CAPTION,
            TEXT_DIMMER,
            false,
        )? + canvas.r(1.2);
    }
    if !clip.muted
        && view
            .studio
            .project
            .as_ref()
            .is_some_and(|project| project.audio_muted(clip))
    {
        y += canvas.text(
            "Its track is muted. Unmute the track to hear this clip.",
            [left, y],
            right - left,
            CAPTION,
            TEXT_DIMMER,
            false,
        )? + canvas.r(1.2);
    }
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(4.0)],
        if clip.muted {
            "Clip muted: On"
        } else {
            "Clip muted: Off"
        },
        Action::ToggleAudioMute,
        true,
        if clip.muted {
            Variant::Primary
        } else {
            Variant::Secondary
        },
    )?;
    y += canvas.r(5.6);
    let button = canvas.r(3.6);
    let duration = clip.span().ok().map(|span| span.duration().seconds());
    for &parameter in AudioParameter::ALL {
        let value = if parameter == AudioParameter::Gain {
            format!("{:.0}%", parameter.value(clip) * 100.0)
        } else {
            format!("{:.2} s", parameter.value(clip))
        };
        canvas.text_line(
            &format!("{} {value}", parameter.label()),
            [left, y + canvas.r(0.6)],
            (right - left - button * 2.0 - canvas.r(1.2)).max(0.0),
            CAPTION,
            TEXT_DIMMER,
        )?;
        for (index, (label, direction)) in [("-", -1), ("+", 1)].into_iter().enumerate() {
            let x = right - (2 - index) as f32 * (button + canvas.r(0.4));
            let value = parameter.value(clip);
            let enabled = if direction < 0 {
                value > 0.0
            } else if parameter == AudioParameter::Gain {
                value < recording_studio::audio::MAX_GAIN
            } else {
                duration.is_some_and(|duration| value < duration)
            };
            control(
                canvas,
                view,
                [x, y, x + button, y + button],
                label,
                Action::AdjustAudio {
                    parameter,
                    direction,
                },
                enabled,
                Variant::Secondary,
            )?;
        }
        y += canvas.r(4.8);
    }
    Ok(y + canvas.r(1.2))
}
