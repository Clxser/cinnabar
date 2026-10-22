//! Native timeline curves expose direct time/value handles on selected owners.
use super::*;
use recording_studio::timeline::{KeyKind, TimelineKey};

/// Key handles carry the frozen ruler geometry used by the input owner for an entire gesture.
fn handle(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    window: &Window,
    key: TimelineKey,
    at: MediaTime,
    y: f32,
    row: f32,
    height: f32,
) -> Result<(), PaintError> {
    if row + height <= window.vertical[0] || row >= window.vertical[1] {
        return Ok(());
    }
    let (at, y) = view
        .studio
        .workspace
        .timeline
        .key_drag
        .filter(|(drag, _, _)| *drag == key)
        .map_or((at, y), |(_, at, delta)| (at, y + delta as f32));
    let x = window.x(at);
    let half = canvas.r(0.8).max(5.0);
    if x < window.left || x > window.right {
        return Ok(());
    }
    let bounds = [
        (x - half).max(window.left),
        (y - half).max(window.vertical[0]).max(row),
        (x + half).min(window.right),
        (y + half).min(window.vertical[1]).min(row + height),
    ];
    if bounds[0] >= bounds[2] || bounds[1] >= bounds[3] {
        return Ok(());
    }
    canvas.fill(bounds, [215, 230, 197, 255])?;
    let locked = view
        .studio
        .project
        .as_ref()
        .and_then(|project| recording_studio::tracks::item_track(project, key.item).ok())
        .is_some_and(|track| view.studio.workspace.timeline.locked.contains(&track));
    if !locked {
        canvas.hit(
            MenuAction::Studio(Action::Timeline(TimelineAction::DragKey {
                key,
                left: window.left.round() as i32,
                width: (window.right - window.left).round().max(1.0) as u32,
                height: height.round().max(1.0) as u32,
            })),
            bounds,
        )?;
    }
    Ok(())
}

/// Camera keys retain authored easing and rails while their time can be dragged directly.
pub(super) fn camera(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    window: &Window,
    shot: &recording_studio::camera::CameraShot,
    y: f32,
    height: f32,
) -> Result<(), PaintError> {
    let mut previous = window.left - 16.0;
    for (at, _) in shot.path_key_times() {
        let x = window.x(at);
        if x - previous < 10.0 {
            continue;
        }
        previous = x;
        handle(
            canvas,
            view,
            window,
            TimelineKey {
                item: shot.id,
                kind: KeyKind::Camera,
                at: at.saturating_sub(shot.span.start),
            },
            at,
            y + height * if shot.fov_curve.is_some() { 0.22 } else { 0.5 },
            y,
            height,
        )?;
    }
    if shot.fov_curve.is_some() {
        let range = recording_studio::camera::CAMERA_FOV_RANGE;
        let normalized = |value: f64| (value - range.start) / (range.end - range.start);
        curve(
            canvas,
            window,
            shot.span,
            y + height * 0.4,
            height * 0.6,
            |at| normalized(shot.fov_override(at).expect("admitted FOV curve")),
        )?;
        let mut previous = window.left - 16.0;
        for (at, key) in shot.fov_keys() {
            let x = window.x(at);
            if x - previous < 10.0 {
                continue;
            }
            previous = x;
            handle(
                canvas,
                view,
                window,
                TimelineKey {
                    item: shot.id,
                    kind: KeyKind::CameraFov,
                    at: at.saturating_sub(shot.span.start),
                },
                at,
                y + height * (0.4 + 0.6 * (0.86 - normalized(key.value) as f32 * 0.72)),
                y,
                height,
            )?;
        }
    }
    Ok(())
}

/// Samples curve segments with a bounded screen-space density, independent of authored key count.
fn curve(
    canvas: &mut Canvas<'_>,
    window: &Window,
    span: Span,
    row: f32,
    height: f32,
    sample: impl Fn(MediaTime) -> f64,
) -> Result<(), PaintError> {
    if row + height <= window.vertical[0] || row >= window.vertical[1] {
        return Ok(());
    }
    let left = window.x(span.start).max(window.left);
    let right = window.x(span.end).min(window.right);
    if left >= right {
        return Ok(());
    }
    let steps = ((right - left) / 3.0).ceil().clamp(2.0, 256.0) as u32;
    for index in 0..steps {
        let x = left + (right - left) * index as f32 / steps as f32;
        let fraction = f64::from((x - window.left) / (window.right - window.left));
        let at = MediaTime(
            window
                .start
                .0
                .saturating_add((fraction * window.duration as f64) as u64),
        );
        let y = row + height * (0.86 - sample(at).clamp(0.0, 1.0) as f32 * 0.72);
        let bounds = [
            x,
            (y - 1.0).max(window.vertical[0]),
            (x + 3.0).min(right),
            (y + 1.0).min(window.vertical[1]),
        ];
        if bounds[1] < bounds[3] {
            canvas.fill(bounds, [170, 218, 178, 255])?;
        }
    }
    Ok(())
}

/// Effect amount uses its domain limits, sharing constraints with the inspector and playback.
pub(super) fn effect(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    window: &Window,
    effect: &recording_studio::effect::EffectClip,
    y: f32,
    height: f32,
) -> Result<(), PaintError> {
    let (minimum, maximum) = effect.kind.limits();
    let normalized = |value: f64| (value - minimum) / (maximum - minimum);
    curve(canvas, window, effect.span, y, height, |at| {
        normalized(effect.amount.sample(at.saturating_sub(effect.span.start)))
    })?;
    let mut previous = window.left - 16.0;
    for key in &effect.amount.keys {
        let Ok(at) = effect.span.start.checked_add(key.at) else {
            continue;
        };
        let x = window.x(at);
        if x - previous < 10.0 {
            continue;
        }
        previous = x;
        handle(
            canvas,
            view,
            window,
            TimelineKey {
                item: effect.id,
                kind: KeyKind::Effect,
                at: key.at,
            },
            at,
            y + height * (0.86 - normalized(key.value) as f32 * 0.72),
            y,
            height,
        )?;
    }
    Ok(())
}

/// Speed keys project absolute source time through the clip map; vertical movement edits multiplier.
pub(super) fn speed(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    window: &Window,
    clip: &recording_studio::model::Clip,
    y: f32,
    height: f32,
) -> Result<(), PaintError> {
    let Some(ramp) = &clip.speed_ramp else {
        return Ok(());
    };
    let Ok(map) = clip.time_map() else {
        return Ok(());
    };
    curve(canvas, window, map.span(), y, height, |at| {
        map.rate_at(at).log2() / 8.0 + 0.5
    })?;
    let mut previous = window.left - 16.0;
    for key in &ramp.keys {
        if key.at < clip.source_range.start || key.at > clip.source_range.end {
            continue;
        }
        let Ok(at) = map.timeline_boundary(key.at) else {
            continue;
        };
        let x = window.x(at);
        if x - previous < 10.0 {
            continue;
        }
        previous = x;
        let value =
            (f64::from(key.rate.numerator) / f64::from(key.rate.denominator)).log2() / 8.0 + 0.5;
        handle(
            canvas,
            view,
            window,
            TimelineKey {
                item: clip.id,
                kind: KeyKind::Speed,
                at: key.at,
            },
            at,
            y + height * (0.86 - value as f32 * 0.72),
            y,
            height,
        )?;
    }
    Ok(())
}
