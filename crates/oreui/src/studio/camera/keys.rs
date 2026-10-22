//! A bounded window of stored path keys remains usable even for large camera rails.

use super::*;
use recording_studio::MediaTime;

const VISIBLE_KEYS: usize = 5;

/// Exact key selection exposes incoming easing, frame timing and pose edits in the same inspector.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    shot: &CameraShot,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    let CameraMode::Path {
        keys,
        smooth_position,
    } = &shot.mode
    else {
        return Ok(y);
    };
    let at = view.studio.position;
    let offset = MediaTime(shot.path_offset.0 + at.saturating_sub(shot.span.start).0);
    let visible_start = keys.partition_point(|key| key.at < shot.path_offset);
    let visible_end =
        keys.partition_point(|key| key.at.0 <= shot.path_offset.0 + shot.span.duration().0);
    let selected = keys
        .binary_search_by_key(&offset, |key| key.at)
        .ok()
        .filter(|_| at >= shot.span.start && at <= shot.span.end);
    let nearby = keys
        .partition_point(|key| key.at <= offset)
        .saturating_sub(1);
    let first = nearby
        .saturating_sub(VISIBLE_KEYS / 2)
        .max(visible_start)
        .min(visible_end.saturating_sub(VISIBLE_KEYS).max(visible_start));
    let last = (first + VISIBLE_KEYS).min(visible_end);
    canvas.text_line(
        &if first == last {
            "NO KEYS IN TRIM".into()
        } else {
            format!(
                "PATH KEYS  {}-{} / {}",
                first - visible_start + 1,
                last - visible_start,
                visible_end - visible_start
            )
        },
        [left, y],
        right - left,
        HEADER5,
        TEXT,
    )?;
    y += canvas.r(3.6);
    for (index, key) in keys[first..last].iter().enumerate() {
        let index = first + index;
        let time = MediaTime(shot.span.start.0 + key.at.0 - shot.path_offset.0);
        let label = format!(
            "{}  {}",
            time_label(time),
            if index == 0 {
                "Start"
            } else {
                key.easing.label()
            }
        );
        control(
            canvas,
            view,
            [left, y, right, y + canvas.r(3.8)],
            &label,
            Action::SelectCameraKey {
                shot: shot.id,
                at: time,
            },
            true,
            if selected == Some(index) {
                Variant::Primary
            } else {
                Variant::Secondary
            },
        )?;
        y += canvas.r(4.6);
    }
    y = pair(
        canvas,
        view,
        [left, y, right, area[3]],
        [
            (
                "Prev key",
                Action::StepCameraKey(-1),
                shot.path_key_times().any(|(time, _)| time < at),
            ),
            (
                "Next key",
                Action::StepCameraKey(1),
                shot.path_key_times().any(|(time, _)| time > at),
            ),
        ],
    )?;
    y = pair(
        canvas,
        view,
        [left, y, right, area[3]],
        [
            (
                if selected.is_some() {
                    "Update key"
                } else {
                    "Add key"
                },
                Action::AddCameraKey,
                view.studio.preview.ready && shot.span.contains(at),
            ),
            (
                "Delete key",
                Action::DeleteCameraKey,
                selected.is_some() && keys.len() > 1,
            ),
        ],
    )?;
    let easing = selected.map_or("Select a key for easing".into(), |index| {
        if index == 0 {
            "First key: no incoming segment".into()
        } else {
            format!("Incoming: {}", keys[index].easing.label())
        }
    });
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(3.8)],
        &easing,
        Action::CycleCameraEasing,
        selected.is_some_and(|index| index > 0),
        Variant::Secondary,
    )?;
    y += canvas.r(4.8);
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(3.8)],
        if shot.path_timing == recording_studio::camera::PathTiming::Distance {
            "Travel: Even speed between keys"
        } else {
            "Travel: Key easing"
        },
        Action::ToggleCameraDistanceTiming,
        keys.len() > 1,
        Variant::Secondary,
    )?;
    y += canvas.r(4.8);
    let gap = canvas.r(0.5);
    let width = (right - left - gap * 3.0) * 0.25;
    for (index, axis) in [
        recording_studio::camera::CameraDragAxis::View,
        recording_studio::camera::CameraDragAxis::X,
        recording_studio::camera::CameraDragAxis::Y,
        recording_studio::camera::CameraDragAxis::Z,
    ]
    .into_iter()
    .enumerate()
    {
        let x = left + index as f32 * (width + gap);
        control(
            canvas,
            view,
            [x, y, x + width, y + canvas.r(3.8)],
            axis.label(),
            Action::SetCameraDragAxis(axis),
            true,
            if view.studio.camera_workspace.drag_axis == axis {
                Variant::Primary
            } else {
                Variant::Secondary
            },
        )?;
    }
    y += canvas.r(4.8);
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(3.8)],
        &format!("Path grid: {}", view.studio.camera_workspace.snap.label()),
        Action::CycleCameraSnap,
        true,
        Variant::Secondary,
    )?;
    y += canvas.r(4.8);
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(3.8)],
        if *smooth_position {
            "Position: Smooth rail"
        } else {
            "Position: Linear rail"
        },
        Action::ToggleCameraSmoothing,
        true,
        Variant::Secondary,
    )?;
    y += canvas.r(4.8);
    y = pair(
        canvas,
        view,
        [left, y, right, area[3]],
        [
            (
                "Key -1 frame",
                Action::MoveCameraKey(-1),
                selected.is_some() && at > shot.span.start,
            ),
            (
                "Key +1 frame",
                Action::MoveCameraKey(1),
                selected.is_some() && at < shot.span.end,
            ),
        ],
    )?;
    canvas.text_line(
        if selected.is_some() {
            "Values below edit the selected key."
        } else {
            "Editing a value adds a key here."
        },
        [left, y],
        right - left,
        CAPTION,
        TEXT_DIMMER,
    )?;
    Ok(y + canvas.r(4.0))
}
