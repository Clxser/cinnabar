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
    let window = Window {
        left,
        right,
        start,
        duration,
    };
    let ruler_bottom = ruler_top + canvas.r(2.8);
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
    let rows = visual_tracks + 4;
    let row_height = canvas.r(3.6);
    let body = [area[0] + pad, ruler_bottom, right, area[3] - pad];
    let scroll = canvas.begin_scroll("studio_timeline", body)?;
    let row_y = |row: usize| ruler_bottom - scroll.offset + row as f32 * row_height;
    for row in 0..rows {
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
            match row - visual_tracks {
                0 => "Cameras",
                1 => "Effects",
                2 => "Audio",
                _ => "Titles",
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
            let label = project
                .sources
                .iter()
                .find(|source| source.id == clip.source)
                .map_or("Clip", |source| source.name.as_str());
            item(
                canvas,
                view,
                &window,
                clip.id,
                span,
                row_y(usize::from(clip.track)),
                row_height,
                label,
                VIDEO,
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
            row_y(visual_tracks),
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
            row_y(visual_tracks + 1),
            row_height,
            "Effect",
            EFFECT,
        )?;
    }
    for audio in &project.audio {
        if let Ok(span) = audio.span() {
            item(
                canvas,
                view,
                &window,
                audio.id,
                span,
                row_y(visual_tracks + 2),
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
            row_y(visual_tracks + 3),
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
