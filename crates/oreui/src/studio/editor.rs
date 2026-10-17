//! Editor composition keeps the preview central, with native controls and actual project metadata.

use super::super::theme::TEXT_DIMMEST;
use super::*;
use recording_studio::MediaTime;

pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), PaintError> {
    let minimum = canvas.r(if area[2] - area[0] > canvas.r(105.0) {
        40.0
    } else {
        46.0
    });
    if area[3] - area[1] < minimum {
        let previous = canvas.settings_scrollbars;
        canvas.settings_scrollbars = true;
        let scroll = canvas.begin_scroll("studio_workspace_compact", area)?;
        let top = area[1] - scroll.offset;
        let result = draw_workspace(
            canvas,
            view,
            [area[0], top, area[2] - canvas.r(1.4), top + minimum],
        );
        let end = canvas.end_scroll(scroll, minimum);
        canvas.settings_scrollbars = previous;
        result?;
        return end;
    }
    draw_workspace(canvas, view, area)
}

fn draw_workspace(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), PaintError> {
    let Some(project) = &view.studio.project else {
        return Ok(());
    };
    let pad = canvas.r(1.2);
    let available = (area[3] - area[1]).max(0.0);
    let timeline_height = (available * 0.42)
        .max(canvas.r(27.0))
        .min((available - canvas.r(12.0)).max(canvas.r(12.0)));
    let timeline_top = (area[3] - timeline_height).max(area[1] + canvas.r(12.0));
    let wide = area[2] - area[0] > canvas.r(105.0);
    let mut upper_top = area[1];
    if !wide {
        let width = ((area[2] - area[0]) - pad * 2.0) / 3.0;
        for (index, (label, panel)) in [
            ("Preview", launcher::menu::studio::EditorPanel::Preview),
            ("Media", launcher::menu::studio::EditorPanel::Media),
            ("Inspector", launcher::menu::studio::EditorPanel::Inspector),
        ]
        .into_iter()
        .enumerate()
        {
            let x = area[0] + index as f32 * (width + pad);
            control(
                canvas,
                view,
                [x, upper_top, x + width, upper_top + canvas.r(4.0)],
                label,
                Action::Panel(panel),
                true,
                if view.studio.workspace.panel == panel {
                    Variant::Primary
                } else {
                    Variant::Secondary
                },
            )?;
        }
        upper_top += canvas.r(4.8);
        let panel_area = [area[0], upper_top, area[2], timeline_top - pad];
        match view.studio.workspace.panel {
            launcher::menu::studio::EditorPanel::Media => {
                media(canvas, view, panel_area)?;
                return timeline::draw(canvas, view, [area[0], timeline_top, area[2], area[3]]);
            }
            launcher::menu::studio::EditorPanel::Inspector => {
                inspector(canvas, view, panel_area)?;
                return timeline::draw(canvas, view, [area[0], timeline_top, area[2], area[3]]);
            }
            launcher::menu::studio::EditorPanel::Preview => {}
        }
    }
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
        upper_top,
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
    } else if let Some(error) = &view.studio.preview.error {
        error.as_str()
    } else if view.studio.preview.loading {
        "Loading preview..."
    } else {
        "Select a playable recording to preview."
    };
    if let Some(icon) = view.studio.preview_icon {
        canvas.fitted_media(icon, viewport)?;
    } else {
        canvas.text_centred(caption, viewport, BODY, TEXT_DIMMEST, false)?;
    }
    if let Some(notice) = &view.studio.preview.notice {
        canvas.text_line(
            notice,
            [viewport[0] + pad, viewport[3] - canvas.r(2.8)],
            (viewport[2] - viewport[0] - pad * 2.0).max(0.0),
            CAPTION,
            TEXT_DIMMER,
        )?;
    }
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
    super::media::draw(
        canvas,
        view,
        [area[0] + pad, area[1] + pad, area[2] - pad, area[3] - pad],
    )
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
    let content_top = y;
    let scroll = canvas.begin_scroll("studio_inspector", [left, y, right, area[3] - pad])?;
    y -= scroll.offset;
    let project = view.studio.project.as_ref().expect("editor has a project");
    if let Some(shot) = project
        .shots
        .iter()
        .find(|shot| Some(shot.id) == view.studio.selected)
    {
        y = super::camera::inspector(canvas, view, [left, y, right, area[3] - pad], shot)?;
    } else if let Some(effect) = project
        .effects
        .iter()
        .find(|effect| Some(effect.id) == view.studio.selected)
    {
        y = super::effects::inspector(canvas, view, [left, y, right, area[3] - pad], effect)?;
    } else if let Some(title) = project
        .titles
        .iter()
        .find(|title| Some(title.id) == view.studio.selected)
    {
        y = super::titles::inspector(canvas, view, [left, y, right, area[3] - pad], title)?;
    } else {
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
        y = super::speed::rates(
            canvas,
            view,
            [left, y, right, area[3] - pad],
            false,
            selected.is_some(),
        )?;
        if let Some(clip) = selected {
            y = super::speed::draw(canvas, view, [left, y, right, area[3] - pad], clip)?;
            y = super::clips::draw(canvas, view, [left, y, right, area[3] - pad], clip)?;
        }
        let native_available = selected
            .and_then(|clip| {
                project
                    .sources
                    .iter()
                    .find(|source| source.id == clip.source)
            })
            .and_then(recording_studio::capture::native_replay_directory)
            .is_some();
        if native_available {
            control(
                canvas,
                view,
                [left, y, right, y + canvas.r(4.4)],
                "Use 3D replay",
                Action::UseNativeReplay,
                native_available,
                Variant::Secondary,
            )?;
        }
        y += canvas.r(6.0);
    }
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(4.4)],
        "Add title",
        Action::AddTitle,
        true,
        Variant::Secondary,
    )?;
    y += canvas.r(6.0);
    y = super::camera::creation(canvas, view, [left, y, right, area[3] - pad])?;
    y += canvas.r(2.0);
    y = super::effects::creation(canvas, view, [left, y, right, area[3] - pad])?;
    let height = (y - content_top + scroll.offset).max(0.0);
    canvas.end_scroll(scroll, height)
}
