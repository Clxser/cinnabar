//! Bounded window painting with frame-snapped scrubbing and stable item hit targets.

use super::*;
use recording_studio::{Id, MediaTime, Span};

const VIDEO: [u8; 4] = [60, 104, 62, 255];
const CAMERA: [u8; 4] = [55, 86, 122, 255];
const EFFECT: [u8; 4] = [95, 95, 50, 255];
const AUDIO: [u8; 4] = [45, 100, 95, 255];
const TITLE: [u8; 4] = [85, 87, 96, 255];
const PLAYHEAD: [u8; 4] = [160, 224, 129, 255];

pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), PaintError> {
    widgets::panel(canvas, area)?;
    let project = view
        .studio
        .project
        .as_ref()
        .expect("timeline has a project");
    let pad = canvas.r(0.8);
    let mut x = area[0] + pad;
    let top = area[1] + pad;
    for (label, action, enabled) in [
        ("Undo", Action::Undo, view.studio.undo.is_some()),
        ("Redo", Action::Redo, view.studio.redo.is_some()),
        ("Split", Action::Split, view.studio.selected.is_some()),
        (
            "Delete",
            Action::DeleteSelected,
            view.studio.selected.is_some(),
        ),
    ] {
        let width = canvas.r(8.4);
        control(
            canvas,
            view,
            [x, top, x + width, top + canvas.r(3.8)],
            label,
            action,
            enabled,
            Variant::Secondary,
        )?;
        x += width + pad;
    }
    let mut right = area[2] - pad;
    for (label, action) in [
        ("+", Action::Zoom(1)),
        ("-", Action::Zoom(-1)),
        ("Later", Action::Pan(1)),
        ("Earlier", Action::Pan(-1)),
    ] {
        let width = canvas.r(if label.len() == 1 { 4.0 } else { 8.4 });
        if right - width <= x {
            break;
        }
        control(
            canvas,
            view,
            [right - width, top, right, top + canvas.r(3.8)],
            label,
            action,
            true,
            Variant::Secondary,
        )?;
        right -= width + pad;
    }
    let ruler_top = top + canvas.r(4.6);
    let label_width = canvas.r(14.0).min((area[2] - area[0]) * 0.25);
    let left = area[0] + label_width;
    let right = area[2] - pad;
    let start = view.studio.visible_start;
    let duration = view.studio.visible_duration.0.max(1);
    let ruler_bottom = ruler_top + canvas.r(2.8);
    let window = Window {
        left,
        right,
        start,
        duration,
        vertical: [ruler_bottom, area[3] - pad],
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
    let visual_tracks = project
        .clips
        .iter()
        .map(|clip| usize::from(clip.track) + 1)
        .max()
        .unwrap_or(1);
    let lanes = &view.studio.lanes;
    let cameras_row = visual_tracks;
    let effects_row = cameras_row + lanes.cameras.rows();
    let audio_row = effects_row + lanes.effects.rows();
    let titles_row = audio_row + lanes.audio.rows();
    let rows = titles_row + lanes.titles.rows();
    let row_height = canvas.r(3.6);
    let body = [area[0] + pad, ruler_bottom, right, area[3] - pad];
    let scroll = canvas.begin_scroll("studio_timeline", body)?;
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
        let label = if row < visual_tracks {
            if row == 0 { "Replay" } else { "Video" }
        } else {
            if row < effects_row {
                "Cameras"
            } else if row < audio_row {
                "Effects"
            } else if row < titles_row {
                "Audio"
            } else {
                "Titles"
            }
        };
        canvas.text_line(
            label,
            [body[0] + pad, y + canvas.r(0.6)],
            label_width - pad * 2.0,
            CAPTION,
            TEXT_DIMMER,
        )?;
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
            effect_keys(
                canvas,
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
    canvas.end_scroll(scroll, rows as f32 * row_height)?;
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
    let left = window.x(span.start).max(window.left);
    let right = window.x(span.end).min(window.right);
    if left >= right {
        return Ok(());
    }
    let pad = canvas.r(0.4);
    let bounds = [left, y + pad, right, y + height - pad];
    canvas.fill(bounds, color)?;
    if view.studio.selected == Some(id) {
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
    canvas.hit(MenuAction::Studio(Action::Select(id)), bounds)
}

/// Dense keys share visible marker columns; previous/next navigation still reaches every authored key.
fn effect_keys(
    canvas: &mut Canvas<'_>,
    window: &Window,
    effect: &recording_studio::effect::EffectClip,
    y: f32,
    height: f32,
) -> Result<(), PaintError> {
    if y + height <= window.vertical[0] || y >= window.vertical[1] {
        return Ok(());
    }
    const MAX_MARKERS: f32 = 256.0;
    let spacing = ((window.right - window.left) / MAX_MARKERS)
        .max(canvas.r(0.6))
        .max(3.0);
    let mut next = window.left;
    for key in &effect.amount.keys {
        let Ok(at) = effect.span.start.checked_add(key.at) else {
            continue;
        };
        let x = window.x(at);
        if x < next || x > window.right {
            continue;
        }
        let half = canvas.r(0.25).max(1.0);
        let bounds = [
            (x - half).max(window.left),
            (y + height * 0.25).max(window.vertical[0]),
            (x + half).min(window.right),
            (y + height * 0.8).min(window.vertical[1]),
        ];
        if bounds[0] < bounds[2] && bounds[1] < bounds[3] {
            canvas.fill(bounds, PLAYHEAD)?;
            canvas.hit(
                MenuAction::Studio(Action::SelectEffectKey {
                    effect: effect.id,
                    at: key.at,
                }),
                bounds,
            )?;
        }
        next = x + spacing;
    }
    Ok(())
}
