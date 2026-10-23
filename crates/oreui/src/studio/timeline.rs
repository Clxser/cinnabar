//! Bounded window painting with frame-snapped scrubbing and stable item hit targets.

use super::*;
use launcher::menu::studio::timeline::TimelineAction;
use recording_studio::tracks::Track;
use recording_studio::{Id, MediaTime, Span};
mod keys;
pub(super) mod layout;

const VIDEO: [u8; 4] = [60, 104, 62, 255];
const CAMERA: [u8; 4] = [55, 86, 122, 255];
const EFFECT: [u8; 4] = [95, 95, 50, 255];
const AUDIO: [u8; 4] = [45, 100, 95, 255];
const TITLE: [u8; 4] = [85, 87, 96, 255];
const PLAYHEAD: [u8; 4] = [160, 224, 129, 255];

/// One responsive toolbar reserves the track viewport instead of wrapping above it.
fn toolbar(canvas: &mut Canvas<'_>, view: &MenuView, area: Bounds) -> Result<f32, PaintError> {
    let metrics = layout::metrics(canvas);
    let pad = metrics.toolbar_pad;
    let height = metrics.toolbar_button;
    if area[1] + metrics.minimum() > area[3] {
        return Ok(area[1] + pad);
    }
    let mut x = area[0] + pad;
    let selected =
        view.studio.selected.is_some() || !view.studio.workspace.timeline.selected.is_empty();
    for (label, action, enabled, active, wide) in [
        (
            "Undo",
            Action::Undo,
            view.studio.undo.is_some(),
            false,
            false,
        ),
        (
            "Redo",
            Action::Redo,
            view.studio.redo.is_some(),
            false,
            false,
        ),
        (
            "Curves",
            Action::Graph(launcher::menu::studio::graph::GraphAction::Toggle),
            true,
            false,
            false,
        ),
        ("Delete", Action::DeleteSelected, selected, false, false),
        (
            "Dup",
            Action::Timeline(TimelineAction::Duplicate),
            selected,
            false,
            false,
        ),
        (
            "Snap",
            Action::Timeline(TimelineAction::ToggleSnapping),
            true,
            view.studio.workspace.timeline.snapping,
            false,
        ),
        ("Fit", Action::FitTimeline, true, false, false),
        ("Split", Action::Split, selected, false, true),
        (
            "Ripple",
            Action::ToggleRipple,
            true,
            view.studio.workspace.ripple,
            true,
        ),
        (
            "Follow",
            Action::ToggleFollowPlayhead,
            true,
            view.studio.workspace.follow_playhead,
            true,
        ),
        ("Playhead", Action::RevealPlayhead, true, false, true),
        ("−", Action::Zoom(-1), true, false, true),
        ("+", Action::Zoom(1), true, false, true),
    ] {
        let width = canvas.r(if label.len() <= 1 {
            3.8
        } else if label.len() > 6 {
            8.0
        } else {
            6.4
        });
        if wide && area[2] - area[0] < canvas.r(86.0) {
            continue;
        }
        if x + width > area[2] - pad {
            break;
        }
        control(
            canvas,
            view,
            [x, area[1] + pad, x + width, area[1] + pad + height],
            label,
            action,
            enabled,
            if active {
                Variant::Primary
            } else {
                Variant::Secondary
            },
        )?;
        x += width + pad;
    }
    Ok(area[1] + height + pad * 2.0)
}

pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), PaintError> {
    if area[2] <= area[0] || area[3] <= area[1] {
        return Ok(());
    }
    widgets::panel(canvas, area)?;
    let project = view
        .studio
        .project
        .as_ref()
        .expect("timeline has a project");
    let pad = canvas.r(0.8);
    let ruler_top = toolbar(canvas, view, area)?;
    let label_width = canvas.r(19.0).min((area[2] - area[0]) * 0.34);
    let left = area[0] + label_width;
    let right = area[2] - pad - canvas.r(1.4);
    let start = view.studio.visible_start;
    let duration = view.studio.visible_duration.0.max(1);
    let metrics = layout::metrics(canvas);
    let ruler_bottom = (ruler_top + metrics.ruler).min(area[3] - pad);
    let body_bottom = area[3] - metrics.footer;
    if right <= left || ruler_bottom >= body_bottom {
        return Ok(());
    }
    let window = Window {
        left,
        right,
        start,
        duration,
        vertical: [ruler_bottom, body_bottom],
    };
    canvas.fill([left, ruler_top, right, ruler_bottom], NEUTRAL90)?;
    for index in 0..=6 {
        let at = MediaTime(start.0.saturating_add(duration * index / 6));
        let x = window.x(at);
        canvas.fill([x, ruler_top, x + 1.0, area[3] - pad], [69, 72, 78, 255])?;
        if index < 6 {
            canvas.text_line(
                &time_label(at),
                [x + pad, ruler_top],
                ((right - left) / 6.0 - pad).max(0.0),
                CAPTION,
                TEXT_DIMMER,
            )?;
        }
    }
    canvas.hit(
        MenuAction::Studio(Action::Scrub {
            left: left.round() as i32,
            width: (right - left).round().max(1.0) as u32,
        }),
        [left, ruler_top, right, ruler_bottom],
    )?;
    let mut marker_column = None;
    for &(at, kind) in view
        .studio
        .lanes
        .visible_markers(start, MediaTime(start.0.saturating_add(duration)))
    {
        let x = window.x(at);
        let column = (x / 4.0).round() as i32;
        if x < left || x > right || marker_column == Some(column) {
            continue;
        }
        marker_column = Some(column);
        canvas.fill(
            [
                (x - 1.0).max(left),
                ruler_top,
                (x + 1.0).min(right),
                ruler_bottom,
            ],
            if kind == recording_studio::model::MarkerKind::Beat {
                [104, 177, 215, 255]
            } else {
                [245, 193, 87, 255]
            },
        )?;
        canvas.hit(
            MenuAction::Studio(Action::Seek(at)),
            [
                (x - 5.0).max(left),
                ruler_top,
                (x + 5.0).min(right),
                ruler_bottom,
            ],
        )?;
    }
    let lanes = &view.studio.lanes;
    let cameras_row = lanes.track_start(Track::Camera);
    let effects_row = lanes.track_start(Track::Effects);
    let audio_row = lanes.track_start(Track::Audio);
    let titles_row = lanes.track_start(Track::Titles);
    let rows = lanes.tracks().len();
    let row_height = metrics.row;
    let body = [
        area[0] + pad,
        ruler_bottom,
        area[2] - pad,
        window.vertical[1],
    ];
    let previous_scrollbars = canvas.settings_scrollbars;
    canvas.settings_scrollbars = true;
    let scroll = canvas.begin_scroll(launcher::menu::studio::TIMELINE_SCROLL_ID, body)?;
    let row_y = |row: usize| ruler_bottom - scroll.offset + row as f32 * row_height;
    let first_row = (scroll.offset.max(0.0) / row_height).floor() as usize;
    let last_row = ((scroll.offset.max(0.0) + body[3] - body[1]) / row_height)
        .ceil()
        .max(0.0) as usize;
    for row in first_row.min(rows)..last_row.min(rows) {
        let y = row_y(row);
        canvas.fill(
            [body[0], y, right, y + row_height],
            if row % 2 == 0 {
                [34, 38, 43, 255]
            } else {
                [39, 43, 49, 255]
            },
        )?;
        let track = lanes.tracks()[row];
        let label = track.label();
        canvas.text_line(
            &label,
            [body[0] + pad, y + canvas.r(0.6)],
            (label_width - canvas.r(7.0)).max(0.0),
            CAPTION,
            TEXT_DIMMER,
        )?;
        for (offset, label, action, active) in [
            (
                canvas.r(6.6),
                "M",
                TimelineAction::ToggleMute(track),
                project.muted_tracks.contains(&track),
            ),
            (
                canvas.r(3.4),
                "L",
                TimelineAction::ToggleLock(track),
                view.studio.workspace.timeline.locked.contains(&track),
            ),
        ] {
            control(
                canvas,
                view,
                [
                    left - offset,
                    y,
                    left - offset + canvas.r(3.0),
                    y + row_height,
                ],
                label,
                Action::Timeline(action),
                true,
                if active {
                    Variant::Primary
                } else {
                    Variant::Secondary
                },
            )?;
        }
    }
    for clip in &project.clips {
        if let Ok(span) = clip.span() {
            let y = row_y(usize::from(clip.track));
            if y + row_height <= body[1]
                || y >= body[3]
                || window.x(span.end) <= window.left
                || window.x(span.start) >= window.right
            {
                continue;
            }
            let label = lanes.source_name(clip.source).unwrap_or("Clip");
            item(
                canvas, view, &window, clip.id, span, y, row_height, label, VIDEO,
            )?;
            if view.studio.selected == Some(clip.id) {
                keys::speed(canvas, view, &window, clip, y, row_height)?;
            }
        }
    }
    for shot in &project.shots {
        item(
            canvas,
            view,
            &window,
            shot.id,
            shot.span,
            row_y(cameras_row + lanes.cameras.row(shot.id).unwrap_or(0)),
            row_height,
            &shot.name,
            CAMERA,
        )?;
        if view.studio.selected == Some(shot.id) {
            keys::camera(
                canvas,
                view,
                &window,
                shot,
                row_y(cameras_row + lanes.cameras.row(shot.id).unwrap_or(0)),
                row_height,
            )?;
        }
    }
    for effect in &project.effects {
        item(
            canvas,
            view,
            &window,
            effect.id,
            effect.span,
            row_y(effects_row + lanes.effects.row(effect.id).unwrap_or(0)),
            row_height,
            effect.kind.label(),
            EFFECT,
        )?;
        if view.studio.selected == Some(effect.id) {
            keys::effect(
                canvas,
                view,
                &window,
                effect,
                row_y(effects_row + lanes.effects.row(effect.id).unwrap_or(0)),
                row_height,
            )?;
        }
    }
    for audio in &project.audio {
        if let Ok(span) = audio.span() {
            item(
                canvas,
                view,
                &window,
                audio.id,
                span,
                row_y(audio_row + lanes.audio.row(audio.id).unwrap_or(0)),
                row_height,
                "Audio",
                AUDIO,
            )?;
        }
    }
    for title in &project.titles {
        item(
            canvas,
            view,
            &window,
            title.id,
            title.span,
            row_y(titles_row + lanes.titles.row(title.id).unwrap_or(0)),
            row_height,
            &title.text,
            TITLE,
        )?;
    }
    let playhead = window.x(view.studio.position);
    if (left..=right).contains(&playhead) {
        canvas.fill(
            [
                playhead,
                ruler_bottom,
                playhead + canvas.r(0.2).max(1.0),
                area[3],
            ],
            PLAYHEAD,
        )?;
    }
    if let Some(at) = view.studio.workspace.timeline.snap_line {
        let x = window.x(at);
        if (left..=right).contains(&x) {
            canvas.fill([x, ruler_bottom, x + 1.0, body_bottom], [245, 193, 87, 255])?;
        }
    }
    canvas.end_scroll(scroll, rows as f32 * row_height)?;
    canvas.settings_scrollbars = previous_scrollbars;
    overview(
        canvas,
        view,
        [left, area[3] - (metrics.footer - pad), right, area[3] - pad],
    )?;
    Ok(())
}

struct Window {
    left: f32,
    right: f32,
    start: MediaTime,
    duration: u64,
    vertical: [f32; 2],
}

impl Window {
    /// Signed differences retain the left edge of clips partially outside the visible window.
    fn x(&self, time: MediaTime) -> f32 {
        let fraction =
            (i128::from(time.0) - i128::from(self.start.0)) as f64 / self.duration as f64;
        self.left + fraction as f32 * (self.right - self.left)
    }
}

/// Clips are culled before text layout and hit registration, then clamped to the visible time window.
#[allow(clippy::too_many_arguments)]
fn item(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    window: &Window,
    id: Id,
    span: Span,
    y: f32,
    height: f32,
    label: &str,
    color: [u8; 4],
) -> Result<(), PaintError> {
    if y + height <= window.vertical[0] || y >= window.vertical[1] {
        return Ok(());
    }
    let span = view
        .studio
        .workspace
        .timeline
        .ghost
        .iter()
        .find(|(item, _)| *item == id)
        .map_or(span, |(_, span)| *span);
    let left = window.x(span.start).max(window.left);
    let right = window.x(span.end).min(window.right);
    if left >= right {
        return Ok(());
    }
    let pad = canvas.r(0.4);
    let bounds = [left, y + pad, right, y + height - pad];
    canvas.fill(bounds, color)?;
    if view.studio.selected == Some(id) || view.studio.workspace.timeline.selected.contains(&id) {
        canvas.frame(bounds, 0.2, PLAYHEAD)?;
    }
    if right - left > canvas.r(7.2) {
        canvas.text_line(
            label,
            [left + canvas.r(0.8), bounds[1] + canvas.r(0.3)],
            right - left - canvas.r(1.6),
            CAPTION,
            TEXT,
        )?;
    }
    let linked = view.studio.project.as_ref().is_some_and(|project| {
        project
            .audio
            .iter()
            .any(|audio| audio.id == id && audio.linked_clip.is_some())
    });
    if linked {
        return canvas.hit(MenuAction::Studio(Action::Select(id)), bounds);
    }
    if view
        .studio
        .project
        .as_ref()
        .and_then(|project| recording_studio::tracks::item_track(project, id).ok())
        .is_some_and(|track| view.studio.workspace.timeline.locked.contains(&track))
    {
        return canvas.hit(MenuAction::Studio(Action::Select(id)), bounds);
    }
    let geometry = (
        window.left.round() as i32,
        (window.right - window.left).round().max(1.0) as u32,
    );
    canvas.hit(
        MenuAction::Studio(Action::DragItem {
            id,
            mode: recording_studio::timeline::DragMode::Move,
            left: geometry.0,
            width: geometry.1,
        }),
        bounds,
    )?;
    let visual = view
        .studio
        .project
        .as_ref()
        .is_some_and(|project| project.clips.iter().any(|clip| clip.id == id));
    if visual && right - left > canvas.r(2.4) {
        let handle = canvas.r(0.8).max(5.0).min((right - left) * 0.25);
        for (x, mode, visible) in [
            (
                left,
                recording_studio::timeline::DragMode::TrimStart,
                window.x(span.start) >= window.left,
            ),
            (
                right - handle,
                recording_studio::timeline::DragMode::TrimEnd,
                window.x(span.end) <= window.right,
            ),
        ] {
            if !visible {
                continue;
            }
            let edge = [x, bounds[1], x + handle, bounds[3]];
            canvas.fill(
                edge,
                if view.studio.selected == Some(id) {
                    PLAYHEAD
                } else {
                    [112, 137, 114, 255]
                },
            )?;
            canvas.hit(
                MenuAction::Studio(Action::DragItem {
                    id,
                    mode,
                    left: geometry.0,
                    width: geometry.1,
                }),
                edge,
            )?;
        }
    }
    Ok(())
}

/// The overview exposes the whole project and a large draggable viewport thumb for long recordings.
fn overview(canvas: &mut Canvas<'_>, view: &MenuView, area: Bounds) -> Result<(), PaintError> {
    let duration = view
        .studio
        .project
        .as_ref()
        .map_or(0, |project| project.duration().0);
    let end = recording_studio::timeline::overview_end(MediaTime(duration)).0;
    let width = (area[2] - area[0]).max(1.0);
    canvas.fill(area, [24, 28, 32, 255])?;
    let left = area[0] + width * (view.studio.visible_start.0 as f64 / end as f64) as f32;
    let right = area[0]
        + width
            * ((view
                .studio
                .visible_start
                .0
                .saturating_add(view.studio.visible_duration.0)
                .min(end)) as f64
                / end as f64) as f32;
    canvas.fill(
        [
            left,
            area[1],
            right.max(left + canvas.r(2.0)).min(area[2]),
            area[3],
        ],
        [99, 113, 124, 255],
    )?;
    let playhead = area[0] + width * (view.studio.position.0 as f64 / end as f64) as f32;
    canvas.fill([playhead, area[1], playhead + 1.0, area[3]], PLAYHEAD)?;
    canvas.hit(
        MenuAction::Studio(Action::TimelineOverview {
            left: area[0].round() as i32,
            width: width.round().max(1.0) as u32,
        }),
        area,
    )
}
