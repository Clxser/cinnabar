//! Camera inspector controls share the saved rig's parameter definitions and native menu primitives.

use recording_studio::camera::{CameraMode, CameraRigKind, CameraShot};

use super::*;

/// New rigs are offered only after a native source has published an admitted camera/target snapshot.
pub(super) fn creation(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    canvas.text_line("ADD CAMERA", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(3.6);
    if view.studio.preview.camera.is_none() {
        canvas.text_line(
            "Use a 3D replay clip first.",
            [left, y],
            right - left,
            CAPTION,
            TEXT_DIMMER,
        )?;
        return Ok(y + canvas.r(4.4));
    }
    y = target(canvas, view, [left, y, right, area[3]])?;
    let pad = canvas.r(0.8);
    let width = (right - left - pad) * 0.5;
    for (index, kind) in CameraRigKind::ALL.iter().copied().enumerate() {
        let x = left + (index % 2) as f32 * (width + pad);
        let row = y + (index / 2) as f32 * canvas.r(4.8);
        let selected = selected_target(view);
        let enabled = view.studio.preview.ready
            && match kind {
                CameraRigKind::Free | CameraRigKind::Path => true,
                CameraRigKind::Player => {
                    selected.is_some_and(|target| target.pose.eye_offset.is_some())
                }
                CameraRigKind::Follow | CameraRigKind::Orbit => selected.is_some(),
            };
        control(
            canvas,
            view,
            [x, row, x + width, row + canvas.r(4.0)],
            kind.label(),
            Action::CreateCamera(kind),
            enabled,
            Variant::Secondary,
        )?;
    }
    Ok(y + CameraRigKind::ALL.len().div_ceil(2) as f32 * canvas.r(4.8))
}

/// Every numeric adjustment is undoable; path adjustments key the exact playhead instead of a neighbour.
pub(super) fn inspector(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    shot: &CameraShot,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    canvas.text_line(&shot.name, [left, y], right - left, BODY, TEXT)?;
    y += canvas.r(3.2);
    canvas.text_line(
        &format!(
            "{} - {}",
            time_label(shot.span.start),
            time_label(shot.span.end)
        ),
        [left, y],
        right - left,
        CAPTION,
        TEXT_DIMMER,
    )?;
    y += canvas.r(3.2);
    let targeted = matches!(
        shot.mode,
        CameraMode::Player { .. } | CameraMode::Follow { .. } | CameraMode::Orbit { .. }
    );
    if targeted {
        y = target(canvas, view, [left, y, right, area[3]])?;
        control(
            canvas,
            view,
            [left, y, right, y + canvas.r(4.0)],
            "Retarget shot",
            Action::RetargetCamera,
            selected_target(view).is_some(),
            Variant::Secondary,
        )?;
        y += canvas.r(5.2);
    }
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
        if view.studio.camera_fine {
            Variant::Primary
        } else {
            Variant::Secondary
        },
    )?;
    y += canvas.r(5.0);
    let at = view.studio.position;
    for &parameter in shot.parameters() {
        let value = shot
            .parameter(parameter, at)
            .map_or_else(|| "--".into(), |value| format!("{value:.2}"));
        let button = canvas.r(3.6);
        canvas.text_line(
            &format!("{}  {value}", parameter.label()),
            [left, y + canvas.r(0.7)],
            (right - left - button * 2.0 - canvas.r(1.2)).max(0.0),
            CAPTION,
            TEXT,
        )?;
        for (index, (label, direction)) in [("-", -1), ("+", 1)].into_iter().enumerate() {
            let x = right - button * (2 - index) as f32;
            control(
                canvas,
                view,
                [x, y, x + button - canvas.r(0.4), y + canvas.r(3.6)],
                label,
                Action::AdjustCamera {
                    parameter,
                    direction,
                },
                shot.span.contains(at),
                Variant::Secondary,
            )?;
        }
        y += canvas.r(4.4);
    }
    if let CameraMode::Path {
        keys,
        smooth_position: _,
    } = &shot.mode
    {
        canvas.text_line(
            &format!("{} path keys", keys.len()),
            [left, y],
            right - left,
            CAPTION,
            TEXT_DIMMER,
        )?;
        y += canvas.r(2.8);
        control(
            canvas,
            view,
            [left, y, right, y + canvas.r(4.0)],
            "Add / update key",
            Action::AddCameraKey,
            view.studio.preview.ready && shot.span.contains(at),
            Variant::Secondary,
        )?;
        y += canvas.r(5.2);
    }
    let width = (right - left - canvas.r(0.8)) * 0.5;
    for (index, (label, action)) in [
        ("Start here", Action::TrimCameraStart),
        ("End here", Action::TrimCameraEnd),
    ]
    .into_iter()
    .enumerate()
    {
        let x = left + index as f32 * (width + canvas.r(0.8));
        control(
            canvas,
            view,
            [x, y, x + width, y + canvas.r(4.0)],
            label,
            action,
            at > shot.span.start && at < shot.span.end,
            Variant::Secondary,
        )?;
    }
    Ok(y + canvas.r(5.2))
}

/// A compact target browser supports large recorded actor sets without painting thousands of controls.
fn target(canvas: &mut Canvas<'_>, view: &MenuView, area: Bounds) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    canvas.text_line(
        "Recorded target",
        [left, y],
        right - left,
        CAPTION,
        TEXT_DIMMER,
    )?;
    y += canvas.r(2.8);
    let name =
        selected_target(view).map_or("No target at this frame", |target| target.name.as_ref());
    let button = canvas.r(3.6);
    let targets = view
        .studio
        .preview
        .camera
        .as_ref()
        .is_some_and(|camera| !camera.targets.is_empty());
    control(
        canvas,
        view,
        [left, y, left + button, y + canvas.r(3.6)],
        "<",
        Action::CycleCameraTarget(-1),
        targets,
        Variant::Secondary,
    )?;
    canvas.text_line(
        name,
        [left + button + canvas.r(0.8), y + canvas.r(0.7)],
        (right - left - button * 2.0 - canvas.r(1.6)).max(0.0),
        CAPTION,
        TEXT,
    )?;
    control(
        canvas,
        view,
        [right - button, y, right, y + canvas.r(3.6)],
        ">",
        Action::CycleCameraTarget(1),
        targets,
        Variant::Secondary,
    )?;
    Ok(y + canvas.r(5.2))
}

/// A stored inspector selection never silently rebinds when an actor's runtime ID is reused.
fn selected_target(view: &MenuView) -> Option<&recording_studio::camera::CameraTarget> {
    let reference = view.studio.camera_target?;
    view.studio
        .preview
        .camera
        .as_ref()?
        .targets
        .iter()
        .find(|target| target.matches(reference))
}
