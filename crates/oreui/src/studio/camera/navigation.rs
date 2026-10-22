//! Small native view tools keep temporary inspection separate from named project views.

use super::*;
use launcher::menu::studio::CameraViewsAction as V;
mod controls;

#[cfg(test)]
mod tests;

/// Saved views share the existing native controls and keep long lists on a bounded page.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    y += canvas.r(1.5);
    canvas.text_line("VIEW TOOLS", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(3.6);
    let ready = view.studio.preview.ready
        && view
            .studio
            .preview
            .camera
            .as_ref()
            .is_some_and(|camera| camera.interactive);
    y = pair(
        canvas,
        view,
        [left, y, right, area[3]],
        [
            (
                "Frame",
                Action::FrameCameraSubject,
                ready && selected_target(view).is_some(),
            ),
            ("Save view...", Action::CameraViews(V::New), ready),
        ],
    )?;
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(3.8)],
        &format!("Guides: {}", view.studio.camera_workspace.guides.label()),
        Action::CycleCameraGuides,
        true,
        Variant::Secondary,
    )?;
    y += canvas.r(5.2);
    y = controls::draw(canvas, view, [left, y, right, area[3]])?;
    let saved = &view.studio.camera_workspace.saved_views;
    if let Some(draft) = &saved.draft {
        canvas.text_line(
            if draft.id.is_some() {
                "Rename view"
            } else {
                "Name this view"
            },
            [left, y],
            right - left,
            CAPTION,
            TEXT_DIMMER,
        )?;
        y += canvas.r(3.0);
        let field = launcher::menu::MenuField::StudioCameraViewName(draft.id);
        widgets::text_field(
            canvas,
            view,
            [left, y, right, y + canvas.r(3.8)],
            &draft.text,
            "View name",
            view.field == Some(field),
            Some(MenuAction::Studio(Action::CameraViews(
                draft.id.map_or(V::New, V::Rename),
            ))),
        )?;
        y += canvas.r(4.8);
        y = pair(
            canvas,
            view,
            [left, y, right, area[3]],
            [
                (
                    "Save name",
                    Action::CameraViews(V::Save),
                    !draft.text.trim().is_empty(),
                ),
                ("Cancel", Action::CameraViews(V::Cancel), true),
            ],
        )?;
    }
    let Some(project) = view.studio.project.as_ref() else {
        return Ok(y);
    };
    canvas.text_line("SAVED VIEWS", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(3.6);
    if project.camera_views.is_empty() {
        return Ok(y
            + canvas.text(
                "Save a named view to reuse its exact framing after reopening this project.",
                [left, y],
                right - left,
                CAPTION,
                TEXT_DIMMER,
                false,
            )?
            + canvas.r(1.2));
    }
    let range = saved.range(project.camera_views.len());
    for bookmark in &project.camera_views[range.clone()] {
        control(
            canvas,
            view,
            [left, y, right, y + canvas.r(3.8)],
            &bookmark.name,
            Action::CameraViews(V::Select(bookmark.id)),
            true,
            if saved.selected == Some(bookmark.id) {
                Variant::Primary
            } else {
                Variant::Secondary
            },
        )?;
        y += canvas.r(4.4);
    }
    if range.len() < project.camera_views.len() {
        y = pair(
            canvas,
            view,
            [left, y, right, area[3]],
            [
                (
                    "Previous",
                    Action::CameraViews(V::Page(-1)),
                    range.start > 0,
                ),
                (
                    "Next",
                    Action::CameraViews(V::Page(1)),
                    range.end < project.camera_views.len(),
                ),
            ],
        )?;
        canvas.text_line(
            &format!(
                "{}-{} of {} views",
                range.start + 1,
                range.end,
                project.camera_views.len()
            ),
            [left, y],
            right - left,
            CAPTION,
            TEXT_DIMMER,
        )?;
        y += canvas.r(3.6);
    }
    if let Some(bookmark) = saved.selected.and_then(|id| {
        project
            .camera_views
            .iter()
            .find(|bookmark| bookmark.id == id)
    }) {
        let active = view
            .studio
            .preview
            .camera
            .as_ref()
            .map(|camera| camera.source);
        let source = project
            .sources
            .iter()
            .find(|source| source.id == bookmark.source);
        let binding_matches = source.is_some_and(|source| bookmark.matches_source(source));
        let available = ready && active == Some(bookmark.source) && binding_matches;
        let status = if !binding_matches {
            "Recording changed. Bind explicitly to reuse this view."
        } else if active != Some(bookmark.source) {
            "Select this view's take to recall it, or bind to the current take."
        } else {
            "Recall changes only the inspection camera."
        };
        y += canvas.text(
            &format!(
                "{}: {status}",
                source.map_or("Missing take", |source| source.name.as_str())
            ),
            [left, y],
            right - left,
            CAPTION,
            TEXT_DIMMER,
            false,
        )? + canvas.r(1.2);
        y = pair(
            canvas,
            view,
            [left, y, right, area[3]],
            [
                (
                    "Recall",
                    Action::CameraViews(V::Recall(bookmark.id)),
                    available,
                ),
                ("Rename", Action::CameraViews(V::Rename(bookmark.id)), true),
            ],
        )?;
        y = pair(
            canvas,
            view,
            [left, y, right, area[3]],
            [
                (
                    "Rebind",
                    Action::CameraViews(V::Rebind(bookmark.id)),
                    ready && !available,
                ),
                (
                    "Delete view",
                    Action::CameraViews(V::Delete(bookmark.id)),
                    true,
                ),
            ],
        )?;
    }
    Ok(y)
}
