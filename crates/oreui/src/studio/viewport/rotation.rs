//! Compact Euler rings are image overlays, usable even at the authored camera's own eye position.

use super::*;
use recording_studio::camera::CameraRotationAxis;

const RING_POINTS: usize = 32;
const RADIUS: f32 = 43.0;

/// Ring geometry is bounded in logical pixels and shared by paint, hit actions and captured drags.
fn ring(bounds: Bounds, axis: CameraRotationAxis) -> Option<[i32; 4]> {
    if bounds[2] - bounds[0] < 180.0 || bounds[3] - bounds[1] < 116.0 {
        return None;
    }
    let center = [bounds[0] + RADIUS + 14.0, bounds[1] + RADIUS + 15.0];
    let radius = match axis {
        CameraRotationAxis::Yaw => [RADIUS, RADIUS * 0.34],
        CameraRotationAxis::Pitch => [RADIUS * 0.34, RADIUS],
        CameraRotationAxis::Roll => [RADIUS * 0.77; 2],
    };
    Some(
        [
            center[0] - radius[0],
            center[1] - radius[1],
            center[0] + radius[0],
            center[1] + radius[1],
        ]
        .map(|value| value.round() as i32),
    )
}

/// A fixed-size widget exposes authored yaw/world-Y, pitch/local-right and roll/view-forward.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    bounds: Bounds,
    shot: Option<&CameraShot>,
) -> Result<(), PaintError> {
    let active = view.studio.camera_workspace.rotation;
    if view
        .studio
        .workspace
        .timeline
        .locked
        .contains(&recording_studio::tracks::Track::Camera)
    {
        return hint(
            canvas,
            bounds,
            "Unlock the Camera track before rotating a shot.",
        );
    }
    let shot = shot.filter(|shot| Some(shot.id) == view.studio.selected);
    let at = view.studio.position;
    let Some((shot, pose)) =
        shot.and_then(|shot| shot.rotation_pose(at).ok().map(|pose| (shot, pose)))
    else {
        return hint(
            canvas,
            bounds,
            "Select a Free shot or an existing Path key to rotate.",
        );
    };
    if active.is_none() && ring(bounds, CameraRotationAxis::Yaw).is_none() {
        return hint(
            canvas,
            bounds,
            "Enlarge the viewer for rotation rings; numeric controls remain available.",
        );
    }
    let ready = view.studio.preview.ready || active.is_some();
    if bounds[2] - bounds[0] > 10.0 && bounds[3] - bounds[1] > 4.0 {
        canvas.fill(
            [
                bounds[0] + 5.0,
                bounds[1] + 2.0,
                (bounds[0] + 240.0).min(bounds[2] - 5.0),
                (bounds[1] + 112.0).min(bounds[3] - 2.0),
            ],
            [16, 20, 24, 190],
        )?;
    }
    let mut lines = Lines::default();
    let mut label = None;
    for axis in CameraRotationAxis::ALL {
        let frozen = active
            .filter(|active| active.shot == shot.id && active.at == at && active.axis == axis);
        let Some(geometry) = frozen
            .map(|active| active.bounds)
            .or_else(|| ring(bounds, axis))
        else {
            continue;
        };
        let ellipse = geometry.map(|value| value as f32);
        let center = [
            (ellipse[0] + ellipse[2]) * 0.5,
            (ellipse[1] + ellipse[3]) * 0.5,
        ];
        let radius = [
            (ellipse[2] - ellipse[0]) * 0.5,
            (ellipse[3] - ellipse[1]) * 0.5,
        ];
        let action = Action::CameraRotationControl {
            shot: shot.id,
            at,
            axis,
            bounds: geometry,
        };
        let hovered = view.hovered == Some(MenuAction::Studio(action));
        let selected = frozen.is_some();
        let color = if !ready {
            TEXT_DIMMER
        } else if selected || hovered {
            TEXT
        } else {
            match axis {
                CameraRotationAxis::Yaw => [110, 220, 140, 230],
                CameraRotationAxis::Pitch => [245, 120, 120, 230],
                CameraRotationAxis::Roll => [120, 175, 250, 230],
            }
        };
        let point = |angle: f32| {
            [
                center[0] + radius[0] * angle.cos(),
                center[1] + radius[1] * angle.sin(),
            ]
        };
        for step in 0..RING_POINTS {
            let [start, end] = [step, step + 1]
                .map(|step| point(step as f32 * std::f32::consts::TAU / RING_POINTS as f32));
            let normalized = [start, end].map(|point| {
                [
                    f64::from((point[0] - bounds[0]) / (bounds[2] - bounds[0])),
                    f64::from((point[1] - bounds[1]) / (bounds[3] - bounds[1])),
                ]
            });
            lines.push(
                bounds,
                normalized,
                color,
                if selected {
                    3.0
                } else if hovered {
                    2.5
                } else {
                    1.3
                },
            );
            // Small segment targets follow the actual ellipse, leaving its interior available for navigation.
            let middle = [(start[0] + end[0]) * 0.5, (start[1] + end[1]) * 0.5];
            if ready {
                hit(canvas, bounds, action, middle, 4.0)?;
            }
        }
        let angle = match axis {
            CameraRotationAxis::Yaw => std::f32::consts::FRAC_PI_2,
            CameraRotationAxis::Pitch => std::f32::consts::PI,
            CameraRotationAxis::Roll => -std::f32::consts::FRAC_PI_4,
        };
        let handle = point(angle);
        if handle[0] >= bounds[0]
            && handle[0] <= bounds[2]
            && handle[1] >= bounds[1]
            && handle[1] <= bounds[3]
        {
            canvas.fill(
                square(bounds, handle, if selected { 4.0 } else { 3.0 }),
                color,
            )?;
            canvas.frame(square(bounds, handle, 5.0), 0.2, color)?;
        }
        if ready {
            hit(canvas, bounds, action, handle, 9.0)?;
        }
        let value = format!("{} {:.1}°", axis.label(), axis.value(pose));
        let position = match axis {
            CameraRotationAxis::Yaw => [center[0] + RADIUS + 10.0, center[1] - 25.0],
            CameraRotationAxis::Pitch => [center[0] + RADIUS + 10.0, center[1] - 5.0],
            CameraRotationAxis::Roll => [center[0] + RADIUS + 10.0, center[1] + 15.0],
        };
        let width = canvas.measure(&value, CAPTION)?;
        if position[0] < bounds[2]
            && position[1] >= bounds[1]
            && position[1] + canvas.r(CAPTION.line) <= bounds[3]
        {
            canvas.text_line(
                &value,
                position,
                (bounds[2] - position[0]).max(0.0),
                CAPTION,
                color,
            )?;
            canvas.hit(
                MenuAction::Studio(Action::EditCameraValue(axis.parameter())),
                [
                    position[0],
                    position[1],
                    (position[0] + width).min(bounds[2]),
                    position[1] + canvas.r(CAPTION.line),
                ],
            )?;
        }
        if hovered || selected {
            label = Some(axis);
        }
    }
    lines.draw(canvas, bounds)?;
    if let Some(axis) = label {
        hint(
            canvas,
            bounds,
            &format!(
                "{} · {} · release to save · Esc cancels",
                axis.label(),
                axis.basis()
            ),
        )?;
    } else if !ready {
        hint(canvas, bounds, "Updating replay view…")?;
    } else {
        hint(
            canvas,
            bounds,
            "Drag a ring · click an angle for a precise value",
        )?;
    }
    Ok(())
}

/// Captured geometry may extend outside a resized image; only its visible positive hit area is admitted.
fn hit(
    canvas: &mut Canvas<'_>,
    bounds: Bounds,
    action: Action,
    point: [f32; 2],
    radius: f32,
) -> Result<(), PaintError> {
    let target = square(bounds, point, radius);
    if target[2] > target[0] && target[3] > target[1] {
        canvas.hit(MenuAction::Studio(action), target)?;
    }
    Ok(())
}

/// Compact instructions occupy the image's top edge and never create another scroll panel.
fn hint(canvas: &mut Canvas<'_>, bounds: Bounds, text: &str) -> Result<(), PaintError> {
    if bounds[2] - bounds[0] <= 16.0 || bounds[3] - bounds[1] < canvas.r(CAPTION.line) + 4.0 {
        return Ok(());
    }
    canvas.fill(
        [
            bounds[0] + 5.0,
            bounds[1] + 2.0,
            bounds[2] - 5.0,
            bounds[1] + canvas.r(CAPTION.line) + 4.0,
        ],
        [16, 20, 24, 210],
    )?;
    canvas.text_line(
        text,
        [bounds[0] + 8.0, bounds[1] + 4.0],
        (bounds[2] - bounds[0] - 16.0).max(0.0),
        CAPTION,
        TEXT_DIMMER,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
