//! Title controls use the existing native font, text field, focus and scroll presentation.

use recording_studio::{
    model::{TitleClip, TitleStyle},
    title::TitleParameter,
};

use super::*;

/// The text draft applies as one undoable edit; styling and timing are ordinary independent edits.
pub(super) fn inspector(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    title: &TitleClip,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    canvas.text_line("TITLE", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(3.6);
    let draft = view
        .studio
        .title_draft
        .as_ref()
        .filter(|(id, _)| *id == title.id)
        .map(|(_, text)| text.as_ref());
    widgets::text_area(
        canvas,
        view,
        [left, y, right, y + canvas.r(11.6)],
        draft.unwrap_or(&title.text),
        "Enter title text",
        view.field == Some(launcher::menu::MenuField::StudioTitle),
        MenuAction::Studio(Action::EditTitleText),
    )?;
    y += canvas.r(12.4);
    if draft.is_some() {
        y = pair(
            canvas,
            view,
            [left, y, right, area[3]],
            [
                ("Apply", Action::ApplyTitleText),
                ("Cancel", Action::CancelTitleText),
            ],
            true,
        )?;
        canvas.text_line(
            "Enter applies, Shift+Enter adds a line",
            [left, y],
            right - left,
            CAPTION,
            TEXT_DIMMER,
        )?;
        y += canvas.r(3.8);
    }
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(3.8)],
        match title.style {
            TitleStyle::Body => "Font: Body",
            TitleStyle::Heading => "Font: Heading",
        },
        Action::CycleTitleStyle,
        true,
        Variant::Secondary,
    )?;
    y += canvas.r(4.8);
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(3.8)],
        if title.shadow {
            "Text shadow: On"
        } else {
            "Text shadow: Off"
        },
        Action::ToggleTitleShadow,
        true,
        Variant::Secondary,
    )?;
    y += canvas.r(4.8);
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
    for &parameter in TitleParameter::ALL {
        canvas.text_line(
            &format!("{}: {:.3}", parameter.label(), parameter.value(title)),
            [left, y],
            right - left,
            CAPTION,
            TEXT,
        )?;
        y += canvas.r(3.0);
        y = pair(
            canvas,
            view,
            [left, y, right, area[3]],
            [
                (
                    "-",
                    Action::AdjustTitle {
                        parameter,
                        direction: -1,
                    },
                ),
                (
                    "+",
                    Action::AdjustTitle {
                        parameter,
                        direction: 1,
                    },
                ),
            ],
            true,
        )?;
    }
    canvas.text_line(
        &format!(
            "{} - {}",
            time_label(title.span.start),
            time_label(title.span.end)
        ),
        [left, y],
        right - left,
        CAPTION,
        TEXT_DIMMER,
    )?;
    y += canvas.r(3.2);
    for (label, action, enabled) in [
        (
            "Start at playhead",
            Action::TrimTitleStart,
            view.studio.position < title.span.end,
        ),
        (
            "End at playhead",
            Action::TrimTitleEnd,
            view.studio.position > title.span.start,
        ),
    ] {
        control(
            canvas,
            view,
            [left, y, right, y + canvas.r(3.8)],
            label,
            action,
            enabled,
            Variant::Secondary,
        )?;
        y += canvas.r(4.8);
    }
    pair(
        canvas,
        view,
        [left, y, right, area[3]],
        [
            ("Move -1 frame", Action::MoveTitle(-1)),
            ("Move +1 frame", Action::MoveTitle(1)),
        ],
        true,
    )
}

/// Two ordinary controls retain the same keyboard/gamepad focus actions as their pointer targets.
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
            [x, area[1], x + width, area[1] + canvas.r(3.8)],
            label,
            action,
            enabled,
            Variant::Secondary,
        )?;
    }
    Ok(area[1] + canvas.r(4.8))
}
