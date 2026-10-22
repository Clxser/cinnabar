//! A small assist panel keeps rhythm, reusable looks and optional camera proposals explicit.

use super::*;
use launcher::menu::studio::creative::{CreativeAction as Assist, audio_clip};

/// Assists never modify footage automatically; each result has a separate undoable apply action.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    let status = &view.studio.workflow.creative;
    canvas.text_line("Beat markers", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(4.0);
    let audio = view
        .studio
        .project
        .as_ref()
        .and_then(|project| audio_clip(project, view.studio.selected, view.studio.position));
    let audio_name = audio
        .and_then(|clip| {
            view.studio
                .project
                .as_ref()?
                .sources
                .iter()
                .find(|source| source.id == clip.source)
        })
        .map(|source| source.name.as_str());
    y += canvas.text(
        audio_name.unwrap_or("Import music, then select its audio clip."),
        [left, y],
        right - left,
        CAPTION,
        TEXT_DIMMER,
        false,
    )? + canvas.r(1.0);
    button(
        canvas,
        view,
        [left, y, right, y + canvas.r(4.0)],
        if status.analyzing {
            "Cancel rhythm analysis"
        } else {
            "Estimate rhythm"
        },
        if status.analyzing {
            Assist::CancelAnalysis
        } else {
            Assist::DetectBeats
        },
        audio.is_some(),
        true,
    )?;
    y += canvas.r(5.0);
    canvas.text_line(
        &format!(
            "Tempo: {:.1} BPM",
            f64::from(status.tempo_tenths) / f64::from(recording_studio::audio::beats::TEMPO_SCALE)
        ),
        [left, y],
        right - left,
        BODY,
        TEXT,
    )?;
    y += canvas.r(3.4);
    let tempo_gap = canvas.r(0.4);
    let tempo_width = (right - left - tempo_gap * 3.0) * 0.25;
    for (index, (label, step)) in [("-1", -10), ("-0.1", -1), ("+0.1", 1), ("+1", 10)]
        .into_iter()
        .enumerate()
    {
        let x = left + index as f32 * (tempo_width + tempo_gap);
        button(
            canvas,
            view,
            [x, y, x + tempo_width, y + canvas.r(3.8)],
            label,
            Assist::StepTempo(step),
            audio.is_some() && !status.analyzing,
            false,
        )?;
    }
    y += canvas.r(4.6);
    y = stepper(
        canvas,
        view,
        [left, y, right, area[3]],
        &format!("Offset {} ms", status.offset_millis),
        Assist::StepOffset(-10),
        Assist::StepOffset(10),
        audio.is_some() && !status.analyzing,
    )?;
    button(
        canvas,
        view,
        [left, y, right, y + canvas.r(3.8)],
        "Put a beat at playhead",
        Assist::BeatHere,
        audio.is_some() && !status.analyzing,
        false,
    )?;
    y += canvas.r(4.8);
    let gap = canvas.r(0.7);
    let half = (right - left - gap) * 0.5;
    button(
        canvas,
        view,
        [left, y, left + half, y + canvas.r(4.0)],
        "Add grid",
        Assist::ApplyBeatGrid,
        audio.is_some() && !status.analyzing,
        true,
    )?;
    button(
        canvas,
        view,
        [left + half + gap, y, right, y + canvas.r(4.0)],
        "Clear grid",
        Assist::ClearBeatGrid,
        audio.is_some() && !status.analyzing,
        false,
    )?;
    y += canvas.r(5.1);
    if let Some(message) = &status.message {
        y += canvas.text(
            message,
            [left, y],
            right - left,
            CAPTION,
            TEXT_DIMMER,
            false,
        )? + canvas.r(1.4);
    }
    y += canvas.text("Tempo grids are estimates. Listen to the music, align the first beat, then add markers. Existing gameplay markers stay intact.", [left, y], right - left, CAPTION, TEXT_DIMMER, false)? + canvas.r(3.0);

    canvas.text_line("Shot suggestions", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(4.0);
    y += canvas.text("Position the 3D view and optionally select a player. Suggestions use the clear replay range at the playhead.", [left, y], right - left, CAPTION, TEXT_DIMMER, false)? + canvas.r(1.2);
    let native = view.studio.preview.ready
        && view
            .studio
            .preview
            .camera
            .as_ref()
            .is_some_and(|camera| camera.interactive);
    button(
        canvas,
        view,
        [left, y, right, y + canvas.r(4.0)],
        "Suggest cameras here",
        Assist::SuggestShots,
        native,
        true,
    )?;
    y += canvas.r(5.4);
    for (index, shot) in status.shots.iter().enumerate() {
        canvas.text_line(&shot.name, [left, y], right - left, BODY, TEXT)?;
        y += canvas.r(3.1);
        canvas.text_line(
            &format!(
                "{} - {}",
                time_label(shot.span.start),
                time_label(shot.span.end)
            ),
            [left, y],
            right - left,
            CAPTION,
            TEXT_DIMMER,
        )?;
        y += canvas.r(3.0);
        button(
            canvas,
            view,
            [left, y, left + half, y + canvas.r(3.8)],
            "View framing",
            Assist::InspectShot(index),
            native,
            false,
        )?;
        button(
            canvas,
            view,
            [left + half + gap, y, right, y + canvas.r(3.8)],
            "Add shot",
            Assist::ApplyShot(index),
            native,
            true,
        )?;
        y += canvas.r(5.3);
    }
    if !status.shots.is_empty() {
        y += canvas.text("Each card is an alternative. Adding one creates a normal editable camera shot. Camera paths can cross walls; inspect the whole move before export.", [left, y], right - left, CAPTION, TEXT_DIMMER, false)? + canvas.r(1.0);
    }
    Ok(y)
}

/// A labelled value stays readable while separate step targets remain easy to hit.
fn stepper(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    label: &str,
    decrease: Assist,
    increase: Assist,
    enabled: bool,
) -> Result<f32, PaintError> {
    let width = canvas.r(3.8).min((area[2] - area[0]) * 0.2);
    let bottom = area[1] + canvas.r(3.8);
    button(
        canvas,
        view,
        [area[0], area[1], area[0] + width, bottom],
        "-",
        decrease,
        enabled,
        false,
    )?;
    canvas.text_centred(
        label,
        [area[0] + width, area[1], area[2] - width, bottom],
        CAPTION,
        TEXT,
        false,
    )?;
    button(
        canvas,
        view,
        [area[2] - width, area[1], area[2], bottom],
        "+",
        increase,
        enabled,
        false,
    )?;
    Ok(bottom + canvas.r(0.8))
}

/// Uses the same focus, disabled and action primitives as the other native editor panels.
fn button(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    label: &str,
    action: Assist,
    enabled: bool,
    primary: bool,
) -> Result<(), PaintError> {
    control(
        canvas,
        view,
        area,
        label,
        Action::Creative(action),
        enabled,
        if primary {
            Variant::Primary
        } else {
            Variant::Secondary
        },
    )
}
