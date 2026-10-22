//! Camera inspector controls share the saved rig's parameter definitions and native menu primitives.

use recording_studio::camera::{
    CameraMode, CameraParameter, CameraRigKind, CameraShot, CameraTargetRef,
};

use super::*;

mod fov;
mod keys;
mod navigation;
mod subjects;

/// New rigs are offered only after a native source has published an admitted camera/target snapshot.
pub(super) fn creation(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    if view.studio.camera_workspace.tools == launcher::menu::studio::CameraTools::Views {
        return navigation::draw(canvas, view, area);
    }
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
                CameraRigKind::Track | CameraRigKind::Follow | CameraRigKind::Orbit => {
                    selected.is_some()
                }
            };
        control(
            canvas,
            view,
            [x, row, x + width, row + canvas.r(4.0)],
            match kind {
                CameraRigKind::Player => "Player POV",
                CameraRigKind::Track => "Track",
                _ => kind.label(),
            },
            Action::CreateCamera(kind),
            enabled,
            Variant::Secondary,
        )?;
    }
    y += CameraRigKind::ALL.len().div_ceil(2) as f32 * canvas.r(4.8);
    if selected_target(view).is_some_and(|target| target.pose.eye_offset.is_none()) {
        for line in [
            "No recorded native eye.",
            "POV unavailable for this subject.",
            "Use Track, Follow or Orbit.",
        ] {
            canvas.text_line(line, [left, y], right - left, CAPTION, TEXT_DIMMER)?;
            y += canvas.r(3.2);
        }
    }
    Ok(y)
}

/// Numeric edits are undoable; whole-rail pose placement requires an explicit timing switch.
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
        CameraMode::Player { .. }
            | CameraMode::Track { .. }
            | CameraMode::Follow { .. }
            | CameraMode::Orbit { .. }
    );
    if targeted {
        let runtime_id = match &shot.mode {
            CameraMode::Player { actor, .. }
            | CameraMode::Follow { actor, .. }
            | CameraMode::Orbit { actor, .. } => *actor,
            CameraMode::Track { actor, .. } => *actor,
            _ => unreachable!(),
        };
        let reference = CameraTargetRef {
            runtime_id,
            lifetime: shot.target_lifetime,
        };
        let subject = view.studio.preview.camera.as_ref().and_then(|preview| {
            preview
                .targets
                .iter()
                .find(|target| target.matches(reference))
        });
        canvas.text_line(
            &subject.map_or_else(
                || "Subject unavailable at this frame".into(),
                |target| format!("Subject: {}", target.name),
            ),
            [left, y],
            right - left,
            CAPTION,
            TEXT_DIMMER,
        )?;
        y += canvas.r(3.4);
        y = target(canvas, view, [left, y, right, area[3]])?;
        control(
            canvas,
            view,
            [left, y, right, y + canvas.r(4.0)],
            "Retarget shot",
            Action::RetargetCamera,
            view.studio.preview.ready
                && selected_target(view).is_some_and(|target| {
                    shot.kind() != CameraRigKind::Player || target.pose.eye_offset.is_some()
                }),
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
    if shot.kind() == CameraRigKind::Path {
        y = keys::draw(canvas, view, [left, y, right, area[3]], shot)?;
    }
    for &parameter in shot.parameters() {
        y = numeric(canvas, view, [left, y, right, area[3]], shot, parameter)?;
    }
    y = fov::draw(canvas, view, [left, y, right, area[3]], shot)?;
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

/// Decimal input and coarse/fine nudges share the same validated camera parameter edit.
fn numeric(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    shot: &CameraShot,
    parameter: CameraParameter,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    let at = view.studio.position;
    let enabled = at >= shot.span.start
        && at <= shot.span.end
        && shot.ensure_parameter_edit(at, parameter).is_ok();
    let label = if shot.kind() == CameraRigKind::Follow
        && matches!(
            parameter,
            CameraParameter::X | CameraParameter::Y | CameraParameter::Z
        ) {
        format!("Offset {}", parameter.label())
    } else {
        parameter.label().into()
    };
    canvas.text_line(&label, [left, y], right - left, CAPTION, TEXT_DIMMER)?;
    y += canvas.r(2.8);
    let value = shot
        .parameter_with_path(
            parameter,
            at,
            view.studio
                .camera_workspace
                .path_cache
                .as_ref()
                .filter(|cache| cache.shot == shot.id)
                .map(|cache| cache.path.as_ref()),
        )
        .map_or_else(|| "--".into(), |value| format!("{value:.3}"));
    let draft = view
        .studio
        .camera_draft
        .as_ref()
        .filter(|(id, time, field, _)| *id == shot.id && *time == at && *field == parameter)
        .map(|(_, _, _, value)| value.as_ref());
    let button = canvas.r(3.6);
    widgets::text_field(
        canvas,
        view,
        [
            left,
            y,
            right - button * 2.0 - canvas.r(0.8),
            y + canvas.r(3.8),
        ],
        draft.unwrap_or(&value),
        parameter.label(),
        draft.is_some() && view.field == Some(launcher::menu::MenuField::StudioCamera(parameter)),
        enabled.then_some(MenuAction::Studio(Action::EditCameraValue(parameter))),
    )?;
    for (index, (label, direction)) in [("-", -1), ("+", 1)].into_iter().enumerate() {
        let x = right - button * (2 - index) as f32;
        control(
            canvas,
            view,
            [x, y, x + button - canvas.r(0.4), y + canvas.r(3.8)],
            label,
            Action::AdjustCamera {
                parameter,
                direction,
            },
            enabled,
            Variant::Secondary,
        )?;
    }
    y += canvas.r(4.8);
    if draft.is_some() {
        y = pair(
            canvas,
            view,
            [left, y, right, area[3]],
            [
                ("Apply", Action::ApplyCameraValue, enabled),
                ("Cancel", Action::CancelCameraValue, true),
            ],
        )?;
    }
    Ok(y)
}

/// Paired inspector actions retain the native focus and disabled-control treatment.
fn pair(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    actions: [(&str, Action, bool); 2],
) -> Result<f32, PaintError> {
    let width = (area[2] - area[0] - canvas.r(0.8)) * 0.5;
    for (index, (label, action, enabled)) in actions.into_iter().enumerate() {
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

/// The source-owned subject browser shares query admission with selection and native focus.
fn target(canvas: &mut Canvas<'_>, view: &MenuView, area: Bounds) -> Result<f32, PaintError> {
    subjects::draw(canvas, view, area)
}

/// A filtered selection always resolves its captured spawn reference rather than its name.
fn selected_target(view: &MenuView) -> Option<&recording_studio::camera::CameraTarget> {
    subjects::selected(view)
}
