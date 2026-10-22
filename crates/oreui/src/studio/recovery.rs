//! Recovery is an explicit full-height view; restoring never silently replaces a saved project file.

use super::*;

/// A failed recovery write stays visible beside the editor until retried; it never claims edits were saved.
pub(super) fn warning(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<f32, PaintError> {
    let Some(error) = view
        .studio
        .workflow
        .recovery
        .error
        .as_ref()
        .filter(|_| !view.studio.workflow.recovery.visible)
    else {
        return Ok(area[1]);
    };
    let pad = canvas.r(0.8);
    let height = canvas.r(4.0);
    let button = canvas.r(10.0).min((area[2] - area[0]) * 0.24);
    canvas.fill(
        [area[0], area[1], area[2], area[1] + height],
        [88, 35, 35, 255],
    )?;
    let right = area[2] - button * 2.0 - pad * 2.0;
    canvas.text_line(
        &format!("Recovery needs attention: {error}"),
        [area[0] + pad, area[1] + pad],
        (right - area[0] - pad * 2.0).max(0.0),
        CAPTION,
        TEXT,
    )?;
    control(
        canvas,
        view,
        [right, area[1], right + button, area[1] + height],
        "Retry",
        Action::RetryAutosave,
        !view.studio.workflow.recovery.busy,
        Variant::Neutral,
    )?;
    control(
        canvas,
        view,
        [
            right + button + pad,
            area[1],
            area[2] - pad,
            area[1] + height,
        ],
        "Recovery",
        Action::ShowRecovery,
        !view.studio.workflow.recovery.busy,
        Variant::Secondary,
    )?;
    Ok(area[1] + height + pad)
}

/// Shows bounded versioned recovery candidates, including damaged copies with an actionable notice.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), PaintError> {
    widgets::panel(canvas, area)?;
    let recovery = &view.studio.workflow.recovery;
    let pad = canvas.r(1.4);
    let [left, top, right, bottom] = [area[0] + pad, area[1] + pad, area[2] - pad, area[3] - pad];
    if right <= left || bottom <= top {
        return Ok(());
    }
    let button = canvas.r(12.0).min((right - left) * 0.3);
    canvas.text_line(
        "Project recovery",
        [left, top],
        (right - left - button * 2.0 - pad * 2.0).max(0.0),
        HEADER5,
        TEXT,
    )?;
    control(
        canvas,
        view,
        [
            right - button * 2.0 - pad,
            top,
            right - button - pad,
            top + canvas.r(3.8),
        ],
        "Refresh",
        Action::ScanRecovery,
        !recovery.loading && !recovery.busy,
        Variant::Neutral,
    )?;
    control(
        canvas,
        view,
        [right - button, top, right, top + canvas.r(3.8)],
        "Back",
        Action::HideRecovery,
        !recovery.busy,
        Variant::Secondary,
    )?;
    let mut y = top + canvas.r(5.0);
    y += canvas.text("Restore opens an unsaved copy. Your original project and source media stay intact. Discard removes only that project's recovery copies.", [left,y],right-left,CAPTION,TEXT_DIMMER,false)?+pad;
    if let Some(error) = &recovery.error {
        y += canvas.text(error, [left, y], right - left, CAPTION, TEXT, false)? + pad;
        control(
            canvas,
            view,
            [left, y, left + canvas.r(20.0), y + canvas.r(3.8)],
            "Retry autosave",
            Action::RetryAutosave,
            !recovery.busy,
            Variant::Secondary,
        )?;
        y += canvas.r(5.0);
    }
    if recovery.loading {
        canvas.text_line(
            "Finding recovery copies...",
            [left, y],
            right - left,
            BODY,
            TEXT_DIMMER,
        )?;
        return Ok(());
    }
    if recovery.entries.is_empty() {
        canvas.text_line(
            "No recovery copies found.",
            [left, y],
            right - left,
            BODY,
            TEXT_DIMMER,
        )?;
        return Ok(());
    }
    if y >= bottom {
        return Ok(());
    }
    let old = canvas.settings_scrollbars;
    canvas.settings_scrollbars = true;
    let scroll = canvas.begin_scroll("studio_recovery", [left, y, right, bottom])?;
    let start = y - scroll.offset;
    let row_height = canvas.r(12.4);
    for (index, entry) in recovery.entries.iter().enumerate() {
        let row = start + index as f32 * row_height;
        if row + row_height < y || row > bottom {
            continue;
        }
        canvas.text_line(&entry.name, [left, row], right - left, BODY, TEXT)?;
        let detail = entry.notice.clone().unwrap_or_else(|| {
            format!(
                "Revision {} / {} recovery versions",
                entry.revision, entry.versions
            )
        });
        canvas.text_line(
            &detail,
            [left, row + canvas.r(2.7)],
            right - left,
            CAPTION,
            TEXT_DIMMER,
        )?;
        let actions_y = row + canvas.r(6.0);
        let width = canvas.r(17.0).min((right - left - pad) * 0.5);
        control(
            canvas,
            view,
            [left, actions_y, left + width, actions_y + canvas.r(3.8)],
            "Restore copy",
            Action::RestoreRecovery(index),
            entry.recoverable && !recovery.busy,
            Variant::Primary,
        )?;
        control(
            canvas,
            view,
            [
                left + width + pad,
                actions_y,
                left + width * 2.0 + pad,
                actions_y + canvas.r(3.8),
            ],
            "Discard copies",
            Action::DiscardRecovery(index),
            !recovery.busy,
            Variant::Destructive,
        )?;
        canvas.fill(
            [
                left,
                row + row_height - pad,
                right,
                row + row_height - pad + 1.0,
            ],
            [63, 68, 73, 255],
        )?;
    }
    let result = canvas.end_scroll(scroll, recovery.entries.len() as f32 * row_height);
    canvas.settings_scrollbars = old;
    result
}
