//! Independent lens animation remains available on every saved camera rig.

use super::*;

/// A bounded key window exposes exact selection, numeric values, deletion and incoming easing.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    shot: &CameraShot,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    let at = view.studio.position;
    let keys = shot.fov_keys().collect::<Vec<_>>();
    let selected = shot.fov_key_at(at);
    canvas.text_line("FOV ANIMATION", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(3.6);
    canvas.text_line(
        if shot.fov_curve.is_some() {
            "Lens keys override the rig FOV."
        } else {
            "Add keys to animate this rig's lens."
        },
        [left, y],
        right - left,
        CAPTION,
        TEXT_DIMMER,
    )?;
    y += canvas.r(4.0);
    let nearby = keys
        .partition_point(|(time, _)| *time <= at)
        .saturating_sub(1);
    let first = nearby.saturating_sub(2).min(keys.len().saturating_sub(5));
    for &(time, key) in keys.iter().skip(first).take(5) {
        let label = format!(
            "{}  {:.1}°  {}",
            time_label(time),
            key.value,
            key.easing.label()
        );
        control(
            canvas,
            view,
            [left, y, right, y + canvas.r(3.8)],
            &label,
            Action::SelectCameraFovKey {
                shot: shot.id,
                at: time,
            },
            true,
            if time == at {
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
                "Prev FOV",
                Action::StepCameraFovKey(-1),
                keys.iter().any(|(time, _)| *time < at),
            ),
            (
                "Next FOV",
                Action::StepCameraFovKey(1),
                keys.iter().any(|(time, _)| *time > at),
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
                    "Update FOV key"
                } else {
                    "Add FOV key"
                },
                Action::AddCameraFovKey,
                at >= shot.span.start && at <= shot.span.end,
            ),
            (
                "Delete FOV key",
                Action::DeleteCameraFovKey,
                selected.is_some(),
            ),
        ],
    )?;
    let incoming = selected.map_or("Select a FOV key".into(), |key| {
        format!("Incoming: {}", key.easing.label())
    });
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(3.8)],
        &incoming,
        Action::CycleCameraFovEasing,
        selected.is_some_and(|key| {
            shot.fov_curve
                .as_ref()
                .is_some_and(|curve| curve.keys.first().is_some_and(|first| first.at != key.at))
        }),
        Variant::Secondary,
    )?;
    y += canvas.r(4.8);
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(3.8)],
        "Restore rig FOV",
        Action::ClearCameraFov,
        shot.fov_curve.is_some(),
        Variant::Secondary,
    )?;
    Ok(y + canvas.r(5.2))
}
