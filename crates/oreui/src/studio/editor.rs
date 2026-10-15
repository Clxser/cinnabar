//! Editor composition keeps the preview central, with native controls and actual project metadata.

use super::super::theme::TEXT_DIMMEST;
use super::*;
use recording_studio::{MediaTime, PlaybackRate};

pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), PaintError> {
    let Some(project) = &view.studio.project else {
        return Ok(());
    };
    let pad = canvas.r(1.2);
    let timeline_height = ((area[3] - area[1]) * 0.33).max(canvas.r(23.0));
    let timeline_top = (area[3] - timeline_height).max(area[1] + canvas.r(12.0));
    let wide = area[2] - area[0] > canvas.r(105.0);
    let left_width = if wide {
        (area[2] - area[0]) * 0.19
    } else {
        0.0
    };
    let right_width = if wide {
        (area[2] - area[0]) * 0.23
    } else {
        0.0
    };
    let preview = [
        area[0] + left_width,
        area[1],
        area[2] - right_width,
        timeline_top - pad,
    ];
    if wide {
        media(
            canvas,
            view,
            [area[0], area[1], preview[0] - pad, preview[3]],
        )?;
        inspector(
            canvas,
            view,
            [preview[2] + pad, area[1], area[2], preview[3]],
        )?;
    }
    widgets::panel(canvas, preview)?;
    let transport_top = preview[3] - canvas.r(5.6);
    let viewport = [
        preview[0] + pad,
        preview[1] + pad,
        preview[2] - pad,
        transport_top - pad,
    ];
    canvas.fill(viewport, [16, 20, 24, 255])?;
    let caption = if project.sources.is_empty() {
        "Add a recording to begin your edit."
    } else {
        "Replay preview"
    };
    canvas.text_centred(caption, viewport, BODY, TEXT_DIMMEST, false)?;
    let button_width = canvas.r(7.2);
    let center = (preview[0] + preview[2]) * 0.5;
    control(
        canvas,
        view,
        [
            center - button_width * 1.6,
            transport_top,
            center - button_width * 0.6,
            preview[3] - pad,
        ],
        "Prev",
        Action::Step(-1),
        true,
        Variant::Secondary,
    )?;
    control(
        canvas,
        view,
        [
            center - button_width * 0.5,
            transport_top,
            center + button_width * 0.5,
            preview[3] - pad,
        ],
        if view.studio.playing { "Pause" } else { "Play" },
        Action::TogglePlayback,
        project.duration() > MediaTime::ZERO,
        Variant::Primary,
    )?;
    control(
        canvas,
        view,
        [
            center + button_width * 0.6,
            transport_top,
            center + button_width * 1.6,
            preview[3] - pad,
        ],
        "Next",
        Action::Step(1),
        true,
        Variant::Secondary,
    )?;
    if preview[2] - preview[0] > canvas.r(60.0) {
        canvas.text_line(
            &time_label(view.studio.position),
            [preview[0] + pad, transport_top + canvas.r(0.8)],
            canvas.r(15.2),
            CAPTION,
            TEXT_DIMMER,
        )?;
    }
    timeline::draw(canvas, view, [area[0], timeline_top, area[2], area[3]])
}

/// Source rows reflect the open project's immutable media, with no placeholder thumbnails.
fn media(canvas: &mut Canvas<'_>, view: &MenuView, area: Bounds) -> Result<(), PaintError> {
    widgets::panel(canvas, area)?;
    let pad = canvas.r(1.6);
    canvas.text_line(
        "MEDIA",
        [area[0] + pad, area[1] + pad],
        area[2] - area[0] - pad * 2.0,
        HEADER5,
        TEXT,
    )?;
    let Some(project) = &view.studio.project else {
        return Ok(());
    };
    let top = area[1] + canvas.r(5.2);
    let scroll = canvas.begin_scroll(
        "studio_media",
        [area[0] + pad, top, area[2] - pad, area[3] - pad],
    )?;
    for (index, source) in project.sources.iter().enumerate() {
        let y = top - scroll.offset + index as f32 * canvas.r(6.0);
        canvas.text_line(
            &source.name,
            [area[0] + pad, y],
            area[2] - area[0] - pad * 2.0,
            BODY,
            TEXT,
        )?;
        canvas.text_line(
            &time_label(source.duration),
            [area[0] + pad, y + canvas.r(2.6)],
            area[2] - area[0] - pad * 2.0,
            CAPTION,
            TEXT_DIMMER,
        )?;
    }
    canvas.end_scroll(scroll, project.sources.len() as f32 * canvas.r(6.0))?;
    Ok(())
}

/// Inspector operations act on the selected timeline identity and commit through normal undo history.
fn inspector(canvas: &mut Canvas<'_>, view: &MenuView, area: Bounds) -> Result<(), PaintError> {
    widgets::panel(canvas, area)?;
    let pad = canvas.r(1.6);
    let left = area[0] + pad;
    let right = area[2] - pad;
    let mut y = area[1] + pad;
    canvas.text_line("INSPECTOR", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(4.4);
    let project = view.studio.project.as_ref().expect("editor has a project");
    let selected = project
        .clips
        .iter()
        .find(|clip| Some(clip.id) == view.studio.selected);
    canvas.text_line(
        if selected.is_some() {
            "Clip speed"
        } else {
            "Select a clip on the timeline."
        },
        [left, y],
        right - left,
        BODY,
        TEXT_DIMMER,
    )?;
    y += canvas.r(3.2);
    let width = (right - left - canvas.r(1.6)) / 3.0;
    for (index, (label, numerator, denominator)) in [("0.5x", 1, 2), ("1x", 1, 1), ("2x", 2, 1)]
        .into_iter()
        .enumerate()
    {
        let x = left + index as f32 * (width + canvas.r(0.8));
        control(
            canvas,
            view,
            [x, y, x + width, y + canvas.r(4.0)],
            label,
            Action::SetRate(PlaybackRate {
                numerator,
                denominator,
            }),
            selected.is_some(),
            Variant::Secondary,
        )?;
    }
    y += canvas.r(6.0);
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(4.4)],
        "Add title",
        Action::AddTitle,
        true,
        Variant::Secondary,
    )?;
    Ok(())
}
