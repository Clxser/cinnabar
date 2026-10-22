//! Search narrows captured subjects without changing the target bound to an authored camera shot.

use super::*;
use launcher::menu::studio::CameraSubjectsAction as S;

/// A native search field and one bounded result page keep large recorded-player sets usable.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    let [left, mut y, right, _] = area;
    let workspace = &view.studio.camera_workspace;
    canvas.text_line(
        "Recorded subject",
        [left, y],
        right - left,
        CAPTION,
        TEXT_DIMMER,
    )?;
    y += canvas.r(2.8);
    control(
        canvas,
        view,
        [left, y, right, y + canvas.r(3.6)],
        &format!("Subjects: {}", workspace.subject_filter.label()),
        Action::ToggleCameraSubjectFilter,
        true,
        Variant::Secondary,
    )?;
    y += canvas.r(4.4);
    let clear = canvas.measure("Clear", crate::theme::SECONDARY_BUTTON)? + canvas.r(1.6);
    widgets::text_field(
        canvas,
        view,
        [left, y, right - clear - canvas.r(0.4), y + canvas.r(3.6)],
        workspace.subject_search.text(),
        "Search name or entity type",
        view.field == Some(launcher::menu::MenuField::StudioSubjectSearch),
        Some(MenuAction::Studio(Action::CameraSubjects(S::Search))),
    )?;
    control(
        canvas,
        view,
        [right - clear, y, right, y + canvas.r(3.6)],
        "Clear",
        Action::CameraSubjects(S::Clear),
        !workspace.subject_search.text().is_empty(),
        Variant::Secondary,
    )?;
    y += canvas.r(4.4);
    let ready = view.studio.preview.ready;
    let results = workspace.subject_results.as_ref().filter(|results| {
        view.studio.preview.camera.as_ref().is_some_and(|preview| {
            results.is_current(preview, &workspace.subject_search, workspace.subject_filter)
        })
    });
    let any = results.is_some_and(|results| !results.is_empty());
    let name = selected(view).map_or("No matching selection", |target| target.name.as_ref());
    let button = canvas.r(3.6);
    control(
        canvas,
        view,
        [left, y, left + button, y + canvas.r(3.6)],
        "<",
        Action::CycleCameraTarget(-1),
        ready && any,
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
        ready && any,
        Variant::Secondary,
    )?;
    y += canvas.r(4.6);
    if let Some(notice) = selected(view).and_then(|target| target.eye_view_notice()) {
        y += canvas.text(notice, [left, y], right - left, CAPTION, TEXT_DIMMER, false)?
            + canvas.r(1.2);
    }
    if !workspace.subject_search.is_empty() {
        if let Some(results) = results {
            if results.is_empty() {
                y += canvas.text(
                    "No matches in this recorded frame. Clear search or change Subjects.",
                    [left, y],
                    right - left,
                    CAPTION,
                    TEXT_DIMMER,
                    false,
                )? + canvas.r(1.2);
            } else {
                let range = results.range(workspace.subject_first);
                canvas.text_line(
                    &format!(
                        "{}-{} of {} matches",
                        range.start + 1,
                        range.end,
                        results.len()
                    ),
                    [left, y],
                    right - left,
                    CAPTION,
                    TEXT_DIMMER,
                )?;
                y += canvas.r(3.2);
                for index in range.clone() {
                    let Some(target) = results.target(index) else {
                        continue;
                    };
                    let chosen = view
                        .studio
                        .camera_target
                        .is_some_and(|reference| target.matches(reference));
                    control(
                        canvas,
                        view,
                        [left, y, right, y + canvas.r(3.6)],
                        &format!("{}  #{}", target.name, target.runtime_id),
                        Action::CameraSubjects(S::Select {
                            source: view
                                .studio
                                .preview
                                .camera
                                .as_ref()
                                .expect("current result frame")
                                .source,
                            epoch: view
                                .studio
                                .preview
                                .camera
                                .as_ref()
                                .expect("current result frame")
                                .epoch,
                            target: target.reference(),
                        }),
                        ready,
                        if chosen {
                            Variant::Primary
                        } else {
                            Variant::Secondary
                        },
                    )?;
                    y += canvas.r(4.2);
                }
                if range.len() < results.len() {
                    y = pair(
                        canvas,
                        view,
                        [left, y, right, area[3]],
                        [
                            (
                                "Previous",
                                Action::CameraSubjects(S::Page(-1)),
                                range.start > 0,
                            ),
                            (
                                "Next",
                                Action::CameraSubjects(S::Page(1)),
                                range.end < results.len(),
                            ),
                        ],
                    )?;
                }
            }
        } else {
            canvas.text_line(
                "Waiting for this recorded frame...",
                [left, y],
                right - left,
                CAPTION,
                TEXT_DIMMER,
            )?;
            y += canvas.r(3.2);
        }
    }
    if view.studio.preview.camera.as_ref().is_some_and(|camera| {
        camera.targets.iter().any(|target| {
            target.kind == recording_studio::camera::CameraTargetKind::Unknown
                && target.runtime_id != camera.local_actor
        })
    }) {
        y += canvas.text(
            recording_studio::camera::LEGACY_SUBJECT_NOTICE,
            [left, y],
            right - left,
            CAPTION,
            TEXT_DIMMER,
            false,
        )? + canvas.r(1.2);
    }
    Ok(y)
}

/// Search affects only browse eligibility; spawn lifetime remains the exact lookup key.
pub(super) fn selected(view: &MenuView) -> Option<&recording_studio::camera::CameraTarget> {
    let reference = view.studio.camera_target?;
    let camera = view.studio.preview.camera.as_ref()?;
    camera.targets.iter().find(|target| {
        target.matches(reference)
            && view
                .studio
                .camera_workspace
                .subject_matches(target, camera.local_actor)
    })
}

#[cfg(test)]
mod tests;
