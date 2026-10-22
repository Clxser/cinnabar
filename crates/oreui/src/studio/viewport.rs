//! Recorded subjects and saved rails project into the exact fitted replay image, never its letterbox.

use std::sync::Arc;

use recording_studio::{
    MediaTime,
    camera::{CameraDragAxis, CameraProjection, CameraShot, PathControl},
};
use ui::{
    UiBlendMode, UiMesh, UiMeshBatch, UiMeshVertex,
    oreui_theme::{BORDER, PRIMARY_ROLE},
};

use super::*;
use launcher::menu::studio::ViewportTool;

const MAX_VISIBLE_KEYS: usize = 128;
const MAX_VISIBLE_SUBJECTS: usize = 24;
const SEGMENT_POINTS: usize = 9;

/// The caller supplies the actual aspect-fitted image rectangle after painting its media.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    bounds: Bounds,
    visible: Bounds,
) -> Result<(), PaintError> {
    let Some(preview) = view.studio.preview.camera.as_ref() else {
        return Ok(());
    };
    if !preview.interactive {
        return Ok(());
    }
    if !preview
        .viewport
        .iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
        || preview.viewport[2] <= preview.viewport[0]
        || preview.viewport[3] <= preview.viewport[1]
    {
        return Ok(());
    }
    let outer = bounds;
    let bounds = [
        outer[0] + (outer[2] - outer[0]) * preview.viewport[0] as f32,
        outer[1] + (outer[3] - outer[1]) * preview.viewport[1] as f32,
        outer[0] + (outer[2] - outer[0]) * preview.viewport[2] as f32,
        outer[1] + (outer[3] - outer[1]) * preview.viewport[3] as f32,
    ];
    let width = bounds[2] - bounds[0];
    let height = bounds[3] - bounds[1];
    if width < 1.0 || height < 1.0 {
        return Ok(());
    }
    let Some(projection) = CameraProjection::new(preview.render_pose, f64::from(width / height))
    else {
        return Ok(());
    };
    let geometry = bounds.map(|side| side.round() as i32);
    let hit = [
        bounds[0].max(visible[0]),
        bounds[1].max(visible[1]),
        bounds[2].min(visible[2]),
        bounds[3].min(visible[3]),
    ];
    canvas.hit(
        MenuAction::Studio(Action::CameraViewport { bounds: geometry }),
        hit,
    )?;
    let workspace = &view.studio.camera_workspace;
    guides(canvas, bounds, workspace.guides)?;
    let shot = workspace.draft.as_deref().or_else(|| {
        let project = view.studio.project.as_ref()?;
        project
            .shots
            .iter()
            .find(|shot| Some(shot.id) == view.studio.selected)
            .or_else(|| {
                project
                    .shots
                    .iter()
                    .filter(|shot| shot.span.contains(view.studio.position))
                    .max_by_key(|shot| (shot.span.start, shot.id))
            })
    });
    let mut lines = Lines::default();
    if let Some(shot) = shot
        .filter(|shot| workspace_tool(view) == ViewportTool::Path && !shot.path_keys().is_empty())
    {
        path(canvas, view, bounds, &projection, shot, &mut lines)?;
    }
    lines.draw(canvas, bounds)?;
    let targets = if workspace_tool(view) == ViewportTool::Players {
        preview.targets.as_ref()
    } else {
        &[]
    };
    let mut subjects = targets
        .iter()
        .filter(|target| target.is_player_subject(preview.local_actor))
        .filter_map(|target| {
            let eye = target.pose.eye_offset.unwrap_or([0.0; 3]);
            let position = std::array::from_fn(|axis| target.pose.position[axis] + eye[axis]);
            let point = projection
                .project(position)
                .filter(|point| point.iter().all(|axis| (0.0..=1.0).contains(axis)))?;
            let pixel = screen(bounds, point);
            if pixel[0] < visible[0]
                || pixel[0] > visible[2]
                || pixel[1] < visible[1]
                || pixel[1] > visible[3]
            {
                return None;
            }
            let selected = view
                .studio
                .camera_target
                .is_some_and(|reference| target.matches(reference));
            let distance = position
                .into_iter()
                .zip(preview.render_pose.position)
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f64>();
            Some((target, point, selected, distance))
        })
        .collect::<Vec<_>>();
    subjects.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.3.total_cmp(&b.3)));
    subjects.truncate(MAX_VISIBLE_SUBJECTS);
    for (target, point, selected, _) in subjects.into_iter().rev() {
        let point = screen(bounds, point);
        let action = Action::CameraSubject(target.reference());
        let hovered = view.hovered == Some(MenuAction::Studio(action));
        let marker = square(bounds, point, if selected || hovered { 5.0 } else { 3.0 });
        let color = if selected {
            PRIMARY_ROLE.specular_hovered[0]
        } else {
            TEXT_DIMMER
        };
        // Pixel-sized open crosshairs leave the player's skin visible at every GUI density.
        canvas.fill(
            [marker[0], point[1] - 0.5, marker[2], point[1] + 0.5],
            color,
        )?;
        canvas.fill(
            [point[0] - 0.5, marker[1], point[0] + 0.5, marker[3]],
            color,
        )?;
        canvas.hit(MenuAction::Studio(action), square(bounds, point, 9.0))?;
        if hovered {
            canvas.text_line(
                &target.name,
                [marker[2] + canvas.r(0.7), marker[1]],
                (bounds[2] - marker[2] - canvas.r(1.0)).max(0.0),
                CAPTION,
                TEXT,
            )?;
        }
    }
    // Path hit targets follow subjects so a visible rail control always wins overlapping markers.
    if let Some(shot) = shot
        .filter(|shot| workspace_tool(view) == ViewportTool::Path && !shot.path_keys().is_empty())
    {
        controls(canvas, view, bounds, geometry, &projection, shot)?;
        axes(canvas, view, bounds, geometry, &projection, shot)?;
    }
    if workspace.focused {
        canvas.frame(bounds, 0.25, PRIMARY_ROLE.specular_hovered[0])?;
    }
    Ok(())
}

/// Creation tools never implicitly enable crowded scene-selection overlays.
fn workspace_tool(view: &MenuView) -> ViewportTool {
    view.studio.workspace.viewport_tool
}

/// Guides are editor overlays within the fitted media image and never become recorded pixels.
fn guides(
    canvas: &mut Canvas<'_>,
    bounds: Bounds,
    guides: launcher::menu::studio::CameraGuides,
) -> Result<(), PaintError> {
    use launcher::menu::studio::CameraGuides;
    let [left, top, right, bottom] = bounds;
    let color = [230, 235, 240, 75];
    match guides {
        CameraGuides::Off => {}
        CameraGuides::Thirds => {
            for fraction in [1.0 / 3.0, 2.0 / 3.0] {
                let x = left + (right - left) * fraction;
                let y = top + (bottom - top) * fraction;
                canvas.fill([x - 0.5, top, x + 0.5, bottom], color)?;
                canvas.fill([left, y - 0.5, right, y + 0.5], color)?;
            }
        }
        CameraGuides::Safe => {
            for inset in [0.05, 0.1] {
                canvas.frame(
                    [
                        left + (right - left) * inset,
                        top + (bottom - top) * inset,
                        right - (right - left) * inset,
                        bottom - (bottom - top) * inset,
                    ],
                    0.15,
                    color,
                )?;
            }
        }
    }
    Ok(())
}

/// One selected path key exposes world-axis handles; edge-on axes are hidden instead of unstable hits.
fn axes(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    bounds: Bounds,
    geometry: [i32; 4],
    projection: &CameraProjection,
    shot: &CameraShot,
) -> Result<(), PaintError> {
    let Some((_, at)) = view
        .studio
        .camera_workspace
        .key
        .filter(|(id, _)| *id == shot.id)
    else {
        return Ok(());
    };
    let Some(key) = shot.key_at(at) else {
        return Ok(());
    };
    let Some(origin) = projection.project(key.pose.position) else {
        return Ok(());
    };
    let camera = view
        .studio
        .preview
        .camera
        .as_ref()
        .expect("native projection admitted");
    let distance = key
        .pose
        .position
        .into_iter()
        .zip(camera.render_pose.position)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        .sqrt();
    let length = (distance * 0.14).clamp(0.25, 16.0);
    let mut lines = Lines::default();
    for axis in CameraDragAxis::WORLD {
        let index = axis.index().expect("world axis");
        let mut end = key.pose.position;
        end[index] += length;
        let Some(point) = projection
            .project(end)
            .filter(|point| point.iter().all(|v| (0.0..=1.0).contains(v)))
        else {
            continue;
        };
        let start_pixel = screen(bounds, origin);
        let end_pixel = screen(bounds, point);
        if (start_pixel[0] - end_pixel[0]).hypot(start_pixel[1] - end_pixel[1]) < 10.0 {
            continue;
        }
        let color = match axis {
            CameraDragAxis::X => [245, 100, 100, 230],
            CameraDragAxis::Y => [110, 220, 140, 230],
            CameraDragAxis::Z => [110, 165, 250, 230],
            CameraDragAxis::View => unreachable!(),
        };
        if let Some(segment) = projection.segment(key.pose.position, end) {
            lines.push(bounds, segment, color, 2.0);
        }
        canvas.fill(square(bounds, end_pixel, 3.0), color)?;
        canvas.text_line(
            axis.label(),
            [end_pixel[0] + 4.0, end_pixel[1] - 4.0],
            (bounds[2] - end_pixel[0] - 4.0).max(0.0),
            CAPTION,
            color,
        )?;
        canvas.hit(
            MenuAction::Studio(Action::CameraAxisControl {
                shot: shot.id,
                at,
                handle: PathControl::Key,
                axis,
                bounds: geometry,
            }),
            square(bounds, end_pixel, 9.0),
        )?;
    }
    lines.draw(canvas, bounds)
}

/// Reset only changes transient state and remains usable while a replacement image is preparing.
pub(super) fn view_actions(ready: bool) -> [(&'static str, Action, bool); 2] {
    [
        ("Apply", Action::ApplyCameraView, ready),
        ("Reset", Action::ResetCameraView, true),
    ]
}

/// A bounded window around the selected key keeps very large admitted rails within UI geometry limits.
fn key_window(view: &MenuView, shot: &CameraShot) -> std::ops::Range<usize> {
    let keys = shot.path_keys();
    let selected = view
        .studio
        .camera_workspace
        .key
        .filter(|(id, _)| *id == shot.id)
        .map_or(view.studio.position, |(_, at)| at)
        .saturating_sub(shot.span.start);
    let selected = MediaTime(shot.path_offset.0 + selected.0);
    let center = keys
        .partition_point(|key| key.at <= selected)
        .saturating_sub(1);
    let start = center
        .saturating_sub(MAX_VISIBLE_KEYS / 2)
        .min(keys.len().saturating_sub(MAX_VISIBLE_KEYS));
    start..(start + MAX_VISIBLE_KEYS).min(keys.len())
}

/// Rail geometry follows traveled distance; edit markers retain their separate orientation-key clock.
fn geometry_window(view: &MenuView, shot: &CameraShot) -> std::ops::Range<usize> {
    if shot.path_timing == recording_studio::camera::PathTiming::GlobalDistance {
        return view
            .studio
            .camera_workspace
            .path_cache
            .as_ref()
            .filter(|cache| cache.shot == shot.id)
            .map_or(0..0, |cache| {
                cache
                    .path
                    .geometry_window(view.studio.position, MAX_VISIBLE_KEYS)
            });
    }
    let range = key_window(view, shot);
    range.start..range.end.min(shot.path_keys().len().saturating_sub(1))
}

/// Samples the saved evaluator rather than drawing a straight decorative approximation of a curve.
fn path(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    bounds: Bounds,
    projection: &CameraProjection,
    shot: &CameraShot,
    lines: &mut Lines,
) -> Result<(), PaintError> {
    let keys = shot.path_keys();
    let window = key_window(view, shot);
    for index in geometry_window(view, shot) {
        let points = if shot.path_timing != recording_studio::camera::PathTiming::Keyed {
            view.studio
                .camera_workspace
                .path_cache
                .as_ref()
                .filter(|cache| cache.shot == shot.id)
                .and_then(|cache| cache.path.geometry_samples::<SEGMENT_POINTS>(index))
        } else {
            shot.path_geometry_samples::<SEGMENT_POINTS>(index)
        };
        let Some(points) = points else {
            continue;
        };
        for pair in points.windows(2) {
            if let Some(segment) = projection.segment(pair[0], pair[1]) {
                lines.push(bounds, segment, TEXT_DIMMER, 1.2);
            }
        }
    }
    if keys.len() > MAX_VISIBLE_KEYS {
        canvas.text_line(
            &format!("Keys {}–{} of {}", window.start + 1, window.end, keys.len()),
            [bounds[0] + canvas.r(1.0), bounds[1] + canvas.r(5.0)],
            (bounds[2] - bounds[0] - canvas.r(2.0)).max(0.0),
            CAPTION,
            TEXT_DIMMER,
        )?;
    }
    if let Some((id, at)) = view
        .studio
        .camera_workspace
        .key
        .filter(|(id, _)| *id == shot.id)
        && let Some(key) = shot.key_at(at)
    {
        let _ = id;
        for handle in [PathControl::Incoming, PathControl::Outgoing] {
            if let Some(position) = shot.path_control(at, handle)
                && let Some(segment) = projection.segment(key.pose.position, position)
            {
                lines.push(bounds, segment, TEXT, 1.0);
            }
        }
    }
    Ok(())
}

/// Every painted control has a hit action bound to the exact shot, source key time and image geometry.
fn controls(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    bounds: Bounds,
    geometry: [i32; 4],
    projection: &CameraProjection,
    shot: &CameraShot,
) -> Result<(), PaintError> {
    for index in key_window(view, shot) {
        let key = &shot.path_keys()[index];
        let Some(offset) = key
            .at
            .0
            .checked_sub(shot.path_offset.0)
            .filter(|offset| *offset <= shot.span.duration().0)
        else {
            continue;
        };
        let at = MediaTime(shot.span.start.0 + offset);
        let selected = view.studio.camera_workspace.key == Some((shot.id, at));
        for handle in [
            PathControl::Incoming,
            PathControl::Outgoing,
            PathControl::Key,
        ] {
            if handle != PathControl::Key && !selected {
                continue;
            }
            let Some(position) = shot.path_control(at, handle) else {
                continue;
            };
            let Some(point) = projection
                .project(position)
                .filter(|point| point.iter().all(|axis| (0.0..=1.0).contains(axis)))
            else {
                continue;
            };
            let point = screen(bounds, point);
            let size = if handle == PathControl::Key {
                if selected { 5.0 } else { 3.5 }
            } else {
                3.0
            };
            let marker = square(bounds, point, size);
            canvas.fill(
                marker,
                if selected && handle == PathControl::Key {
                    PRIMARY_ROLE.fill
                } else {
                    BORDER
                },
            )?;
            canvas.frame(marker, 0.25, TEXT)?;
            let action = Action::CameraControl {
                shot: shot.id,
                at,
                handle,
                bounds: geometry,
            };
            canvas.hit(MenuAction::Studio(action), square(bounds, point, 8.5))?;
            if handle == PathControl::Key
                && (selected || view.hovered == Some(MenuAction::Studio(action)))
            {
                canvas.text_line(
                    &(index + 1).to_string(),
                    [marker[2] + 2.0, marker[1]],
                    (bounds[2] - marker[2]).max(0.0),
                    CAPTION,
                    TEXT,
                )?;
            }
        }
    }
    Ok(())
}

/// Maps normalized projection coordinates to the image's current logical-pixel rectangle.
fn screen(bounds: Bounds, point: [f64; 2]) -> [f32; 2] {
    [
        bounds[0] + (bounds[2] - bounds[0]) * point[0] as f32,
        bounds[1] + (bounds[3] - bounds[1]) * point[1] as f32,
    ]
}

/// Keeps visible markers and their hit rectangles inside the fitted native image.
fn square(bounds: Bounds, point: [f32; 2], radius: f32) -> Bounds {
    [
        (point[0] - radius).max(bounds[0]),
        (point[1] - radius).max(bounds[1]),
        (point[0] + radius).min(bounds[2]),
        (point[1] + radius).min(bounds[3]),
    ]
}

#[derive(Default)]
struct Lines {
    vertices: Vec<UiMeshVertex>,
    indices: Vec<u32>,
}

impl Lines {
    /// Builds one anti-gap quad per projected line, clamped to the image's normalized bounds.
    fn push(&mut self, bounds: Bounds, segment: [[f64; 2]; 2], color: [u8; 4], thickness: f32) {
        let [a, b] = segment.map(|point| {
            screen(
                [0.0, 0.0, bounds[2] - bounds[0], bounds[3] - bounds[1]],
                point,
            )
        });
        let delta = [b[0] - a[0], b[1] - a[1]];
        let length = delta[0].hypot(delta[1]);
        if length < 0.1 {
            return;
        }
        let normal = [
            -delta[1] / length * thickness * 0.5,
            delta[0] / length * thickness * 0.5,
        ];
        let base = self.vertices.len() as u32;
        for (point, sign) in [(a, -1.0), (a, 1.0), (b, 1.0), (b, -1.0)] {
            self.vertices.push(UiMeshVertex {
                position: [
                    ((point[0] + normal[0] * sign) / (bounds[2] - bounds[0])).clamp(0.0, 1.0),
                    ((point[1] + normal[1] * sign) / (bounds[3] - bounds[1])).clamp(0.0, 1.0),
                ],
                clip_z: 0.0,
                clip_w: 1.0,
                uv: [0.0; 2],
                color,
                model_light: 1.0,
                overlay_color: [0.0; 4],
                style_flags: 0,
                alpha_test: false,
            });
        }
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// One native UI mesh draws the entire bounded rail behind its interactive controls.
    fn draw(mut self, canvas: &mut Canvas<'_>, bounds: Bounds) -> Result<(), PaintError> {
        if self.indices.is_empty() {
            return Ok(());
        }
        for vertex in &mut self.vertices {
            vertex.color[3] = (f32::from(vertex.color[3]) * canvas.alpha).round() as u8;
        }
        let count = self.indices.len() as u32;
        let mesh = UiMesh::new(
            self.vertices.into(),
            self.indices.into(),
            Arc::from([UiMeshBatch {
                texture_page: canvas.solid_page,
                index_range: 0..count,
                blend: UiBlendMode::Alpha,
                depth_test: false,
                depth_write: false,
                alpha_cutoff: None,
            }]),
        )
        .map_err(|_| PaintError::Tree(ui::UiError::DrawIndexOverflow))?;
        canvas.mesh(bounds, Arc::new(mesh))
    }
}

#[cfg(test)]
mod tests;
