//! View tools stay compact and clearly separate temporary session views from saved camera shots.

use super::*;

/// Subject framing, composition guides and bounded session views use the same native camera admission.
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
                "Frame player",
                Action::FrameCameraSubject,
                ready && selected_target(view).is_some(),
            ),
            ("Store view", Action::SaveCameraBookmark, ready),
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
    y += canvas.r(4.8);
    let bookmarks = &view.studio.camera_workspace.bookmarks;
    if !bookmarks.is_empty() {
        canvas.text_line(
            "SESSION VIEWS",
            [left, y],
            right - left,
            CAPTION,
            TEXT_DIMMER,
        )?;
        y += canvas.r(3.2);
        let gap = canvas.r(0.5);
        let width = (right - left - gap * 3.0) * 0.25;
        for index in 0..bookmarks.len() {
            let x = left + (index % 4) as f32 * (width + gap);
            let top = y + (index / 4) as f32 * canvas.r(4.5);
            control(
                canvas,
                view,
                [x, top, x + width, top + canvas.r(3.8)],
                &(index + 1).to_string(),
                Action::RecallCameraBookmark(index),
                ready,
                Variant::Secondary,
            )?;
        }
        y += bookmarks.len().div_ceil(4) as f32 * canvas.r(4.5);
    }
    Ok(y)
}
