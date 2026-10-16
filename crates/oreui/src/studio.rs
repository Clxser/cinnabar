//! Cinnabar's native recording library and timeline workspace.

use launcher::menu::{MenuAction, MenuView, studio::Action};
use ui_engine::engine::screen::PaintError;

use super::{
    paint::{Bounds, Canvas},
    theme::{BODY, CAPTION, HEADER5, NEUTRAL80, NEUTRAL90, TEXT, TEXT_DIMMER},
    widgets::{self, Variant},
};

mod camera;
mod editor;
mod effects;
mod export;
mod library;
mod recording;
mod timeline;
mod titles;

/// Uses the existing OreUI font, focus, button and scroll primitives throughout the workspace.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    size: [f32; 2],
) -> Result<(), PaintError> {
    canvas.fill([0.0, 0.0, size[0], size[1]], NEUTRAL90)?;
    let pad = canvas.r(1.2);
    let header = canvas.r(6.0);
    canvas.fill([0.0, 0.0, size[0], header], NEUTRAL80.fill)?;
    widgets::button(
        canvas,
        view,
        [pad, pad, pad + canvas.r(7.2), header - pad],
        Variant::Neutral,
        "Back",
        Some(MenuAction::AddBack),
    )?;
    let title_left = pad * 2.0 + canvas.r(7.2);
    canvas.text_line(
        &launcher::menu::studio::TITLE.to_uppercase(),
        [title_left, pad],
        size[0] * 0.45 - title_left,
        HEADER5,
        TEXT,
    )?;
    let studio = &view.studio;
    let subtitle = studio
        .project
        .as_ref()
        .map_or("Your recordings and projects", |project| {
            project.name.as_str()
        });
    canvas.text_line(
        subtitle,
        [title_left, pad + canvas.r(2.4)],
        size[0] * 0.45 - title_left,
        CAPTION,
        TEXT_DIMMER,
    )?;
    let status = if studio.saving {
        "Saving..."
    } else if studio.dirty {
        "Unsaved changes"
    } else if studio.file.is_some() {
        "Saved"
    } else {
        ""
    };
    let save_width = canvas.r(9.6);
    let export_width = canvas.r(10.4);
    let library_width = canvas.r(12.0);
    let right = size[0] - pad;
    control(
        canvas,
        view,
        [right - save_width, pad, right, header - pad],
        "Save",
        Action::Save,
        studio.project.is_some() && !studio.saving && !studio.loading,
        Variant::Primary,
    )?;
    let export_right = right - save_width - pad;
    control(
        canvas,
        view,
        [export_right - export_width, pad, export_right, header - pad],
        "Export",
        Action::Export,
        studio
            .project
            .as_ref()
            .is_some_and(|project| project.duration() > recording_studio::MediaTime::ZERO)
            && !studio.loading
            && !studio.export.phase.busy(),
        Variant::Secondary,
    )?;
    control(
        canvas,
        view,
        [
            export_right - export_width - library_width - pad,
            pad,
            export_right - export_width - pad,
            header - pad,
        ],
        if studio.show_library && studio.project.is_some() {
            "Back to edit"
        } else {
            "Library"
        },
        Action::Library,
        true,
        Variant::Secondary,
    )?;
    if size[0] > canvas.r(110.0) {
        canvas.text_line(
            status,
            [size[0] * 0.51, pad + canvas.r(1.0)],
            (size[0] * 0.49 - save_width - export_width - library_width - pad * 5.0).max(0.0),
            CAPTION,
            TEXT_DIMMER,
        )?;
    }
    let mut top = header + pad;
    if let Some(error) = &studio.error {
        canvas.fill(
            [pad, top, size[0] - pad, top + canvas.r(4.4)],
            [88, 35, 35, 255],
        )?;
        canvas.text_line(
            error,
            [pad * 2.0, top + canvas.r(0.6)],
            size[0] - pad * 4.0,
            CAPTION,
            TEXT,
        )?;
        top += canvas.r(5.2);
    }
    if let Some(notice) = &studio.notice {
        canvas.text_line(
            notice,
            [pad, top],
            size[0] - pad * 2.0,
            CAPTION,
            TEXT_DIMMER,
        )?;
        top += canvas.r(3.2);
    }
    top = recording::draw(canvas, view, [pad, top, size[0] - pad, size[1] - pad])?;
    top = export::draw(canvas, view, [pad, top, size[0] - pad, size[1] - pad])?;
    let area = [pad, top, size[0] - pad, size[1] - pad];
    if studio.show_library || studio.project.is_none() {
        library::draw(canvas, view, area)?;
    } else {
        editor::draw(canvas, view, area)?;
    }
    if studio.loading {
        canvas.overlay(size, [0, 0, 0, 150])?;
        canvas.hits.clear();
        canvas.clear_focus_geometry();
        let bounds = [size[0] * 0.3, size[1] * 0.4, size[0] * 0.7, size[1] * 0.6];
        widgets::panel(canvas, bounds)?;
        canvas.text_centred(
            "Opening project...",
            [
                bounds[0],
                bounds[1],
                bounds[2],
                bounds[1] + (bounds[3] - bounds[1]) * 0.5,
            ],
            BODY,
            TEXT,
            false,
        )?;
        control(
            canvas,
            view,
            [
                bounds[0] + pad,
                bounds[3] - canvas.r(5.2),
                bounds[2] - pad,
                bounds[3] - pad,
            ],
            "Cancel",
            Action::CancelOpen,
            true,
            Variant::Secondary,
        )?;
        if studio.capture.phase.busy() {
            control(
                canvas,
                view,
                [
                    bounds[0] + pad,
                    bounds[3] + pad,
                    bounds[2] - pad,
                    bounds[3] + canvas.r(5.2),
                ],
                "Stop recording",
                Action::ToggleRecording,
                studio.capture.phase != recording_studio::capture::CapturePhase::Finishing,
                Variant::Neutral,
            )?;
        }
    }
    if studio.discard.is_some() {
        discard(canvas, view, size)?;
    }
    Ok(())
}

/// A disabled control has no hit or focus target and uses the same OreUI disabled styling as settings.
fn control(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    bounds: Bounds,
    label: &str,
    action: Action,
    enabled: bool,
    variant: Variant,
) -> Result<(), PaintError> {
    widgets::button(
        canvas,
        view,
        bounds,
        variant,
        label,
        enabled.then_some(MenuAction::Studio(action)),
    )
}

/// Document replacement is explicit; navigating away from the workspace retains its open document.
fn discard(canvas: &mut Canvas<'_>, view: &MenuView, size: [f32; 2]) -> Result<(), PaintError> {
    canvas.overlay(size, [0, 0, 0, 180])?;
    canvas.hits.clear();
    canvas.clear_focus_geometry();
    let width = canvas.r(50.0).min(size[0] - canvas.r(4.0));
    let left = (size[0] - width) * 0.5;
    let top = (size[1] - canvas.r(22.0)) * 0.5;
    let bounds = [left, top, left + width, top + canvas.r(22.0)];
    widgets::panel(canvas, bounds)?;
    let pad = canvas.r(2.0);
    canvas.text_line(
        "Discard unsaved changes?",
        [left + pad, top + pad],
        width - pad * 2.0,
        HEADER5,
        TEXT,
    )?;
    canvas.text_line(
        "Save your project before opening another.",
        [left + pad, top + canvas.r(6.0)],
        width - pad * 2.0,
        BODY,
        TEXT_DIMMER,
    )?;
    let y = bounds[3] - canvas.r(6.4);
    let middle = left + width * 0.5;
    control(
        canvas,
        view,
        [left + pad, y, middle - pad * 0.5, bounds[3] - pad],
        "Keep editing",
        Action::CancelDiscard,
        true,
        Variant::Secondary,
    )?;
    control(
        canvas,
        view,
        [middle + pad * 0.5, y, bounds[2] - pad, bounds[3] - pad],
        "Discard",
        Action::ConfirmDiscard,
        true,
        Variant::Destructive,
    )
}

/// Compact media time labels share one formatter across transport, ruler and inspector.
fn time_label(time: recording_studio::MediaTime) -> String {
    let seconds = time.0 / recording_studio::time::MICROS_PER_SECOND;
    let millis = time.0 % recording_studio::time::MICROS_PER_SECOND / 1000;
    format!("{:02}:{:02}.{millis:03}", seconds / 60, seconds % 60)
}
