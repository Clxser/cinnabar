//! Recording and export settings use full-height groups without nested short scroll panes.

use super::*;
use recording_studio::settings::{codec_label, encoder_label};

mod diagnostics;
mod export_range;
mod help;

/// One selected settings group owns the full usable height, with no nested short scroll panes.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), PaintError> {
    use launcher::menu::studio::{SettingsPage, WorkspaceAction};
    widgets::panel(canvas, area)?;
    let pad = canvas.r(1.2);
    let inner = [area[0] + pad, area[1] + pad, area[2] - pad, area[3] - pad];
    if inner[2] <= inner[0] || inner[3] <= inner[1] {
        return Ok(());
    }
    let gap = canvas.r(0.5);
    let pages = SettingsPage::ALL;
    let columns = (((inner[2] - inner[0] + gap) / (canvas.r(13.0) + gap)).floor() as usize)
        .clamp(1, pages.len());
    let width = (inner[2] - inner[0] - gap * (columns - 1) as f32) / columns as f32;
    let row_height = canvas.r(3.8);
    for (index, &page) in pages.iter().enumerate() {
        let x = inner[0] + (index % columns) as f32 * (width + gap);
        let y = inner[1] + (index / columns) as f32 * (row_height + gap);
        control(
            canvas,
            view,
            [x, y, x + width, y + row_height],
            page.label(),
            Action::Workspace(WorkspaceAction::SettingsPage(page)),
            true,
            if view.studio.workspace.settings_page == page {
                Variant::Primary
            } else {
                Variant::Neutral
            },
        )?;
    }
    let rows = pages.len().div_ceil(columns);
    let bounds = [
        inner[0],
        inner[1] + rows as f32 * (row_height + gap) + pad,
        inner[2],
        inner[3],
    ];
    if bounds[3] <= bounds[1] {
        return Ok(());
    }
    let previous = canvas.settings_scrollbars;
    canvas.settings_scrollbars = true;
    let key = format!("studio_settings_{:?}", view.studio.workspace.settings_page);
    let scroll = canvas.begin_scroll(&key, bounds)?;
    let top = bounds[1] - scroll.offset;
    let content = [bounds[0], top, bounds[2], bounds[3]];
    let bottom = match view.studio.workspace.settings_page {
        SettingsPage::Recording => capture(canvas, view, content),
        SettingsPage::Preview => preview(canvas, view, content),
        SettingsPage::Export => output(canvas, view, content),
        SettingsPage::Diagnostics => diagnostics::draw(canvas, view, content),
        SettingsPage::Controls => help::draw(canvas, view, content),
    };
    let result = match bottom {
        Ok(bottom) => canvas.end_scroll(scroll, (bottom - top).max(0.0) + pad),
        Err(error) => Err(error),
    };
    canvas.settings_scrollbars = previous;
    result
}

/// Preview controls affect only the interactive viewer; project output retains its saved settings.
fn preview(canvas: &mut Canvas<'_>, view: &MenuView, area: Bounds) -> Result<f32, PaintError> {
    use launcher::menu::studio::WorkspaceAction;
    let [left, mut y, right, bottom] = area;
    canvas.text_line("Preview", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(4.8);
    row(
        canvas,
        view,
        [left, y, right, bottom],
        &format!(
            "Preview quality: {}",
            view.studio.workspace.preview_quality.label()
        ),
        Action::Workspace(WorkspaceAction::CyclePreviewQuality),
        true,
    )?;
    y += canvas.r(5.0);
    y += canvas.text("Fast uses a smaller picture at up to 30 FPS. Balanced and Full preview at up to 60 FPS. Export retains the project's exact output resolution and frame rate.", [left, y], right-left, CAPTION, TEXT_DIMMER, false)? + canvas.r(2.0);
    row(
        canvas,
        view,
        [left, y, right, bottom],
        if view.studio.workspace.preview_fill {
            "Viewer: Fill (crop edges)"
        } else {
            "Viewer: Fit (whole picture)"
        },
        Action::Workspace(WorkspaceAction::TogglePreviewFill),
        true,
    )?;
    y += canvas.r(5.0);
    row(
        canvas,
        view,
        [left, y, right, bottom],
        &format!(
            "Interface density: {}",
            view.studio.workspace.density.label()
        ),
        Action::Workspace(WorkspaceAction::Density),
        true,
    )?;
    y += canvas.r(5.0);
    row(
        canvas,
        view,
        [left, y, right, bottom],
        view.studio.workspace.timeline_size.label(),
        Action::Workspace(WorkspaceAction::TimelineSize),
        true,
    )?;
    y += canvas.r(5.0);
    Ok(y)
}

/// Capture choices apply to the next take and remain disabled while capture owns those settings.
fn capture(canvas: &mut Canvas<'_>, view: &MenuView, area: Bounds) -> Result<f32, PaintError> {
    let [left, top, right, bottom] = area;
    let mut y = top;
    canvas.text_line("Recording", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(4.8);
    let capture = view.studio.workflow.capture_settings;
    row(
        canvas,
        view,
        [left, y, right, bottom],
        &format!("Capture: {}", capture.mode.label()),
        Action::CycleCaptureMode,
        !view.studio.capture.phase.busy(),
    )?;
    y += canvas.r(5.0);
    row(
        canvas,
        view,
        [left, y, right, bottom],
        &format!("Recording FPS: {}", capture.fps),
        Action::CycleCaptureFps,
        !view.studio.capture.phase.busy(),
    )?;
    y += canvas.r(5.0);
    row(
        canvas,
        view,
        [left, y, right, bottom],
        &format!("Recording size: {}", capture.size.label()),
        Action::CycleCaptureSize,
        !view.studio.capture.phase.busy(),
    )?;
    y += canvas.r(5.0);
    row(
        canvas,
        view,
        [left, y, right, bottom],
        if capture.hand {
            "Record first-person hand: On"
        } else {
            "Record first-person hand: Off"
        },
        Action::ToggleCaptureHand,
        !view.studio.capture.phase.busy(),
    )?;
    y += canvas.r(5.0);
    row(
        canvas,
        view,
        [left, y, right, bottom],
        if capture.hud {
            "Record HUD and chat: On"
        } else {
            "Record HUD and chat: Off"
        },
        Action::ToggleCaptureHud,
        !view.studio.capture.phase.busy(),
    )?;
    y += canvas.r(5.0);
    y += canvas.text("3D replay keeps editable cameras and game audio without encoding video during play. Add video when you also want the recorded camera as a movie. Changes apply to the next take; higher capture FPS records more motion samples.", [left, y], right - left, CAPTION, TEXT_DIMMER, false)? + canvas.r(3.0);
    Ok(y)
}

/// Output choices and destination describe the actual export job queued from the current project.
fn output(canvas: &mut Canvas<'_>, view: &MenuView, area: Bounds) -> Result<f32, PaintError> {
    let [left, top, right, bottom] = area;
    let pad = canvas.r(1.6);
    let mut y = top;
    canvas.text_line("Export", [left, y], right - left, HEADER5, TEXT)?;
    y += canvas.r(4.8);
    if let Some(project) = &view.studio.project {
        y = export_range::draw(canvas, view, [left, y, right, bottom])?;
        let settings = &project.export;
        let enabled = !view.studio.loading;
        for (label, action) in [
            (
                format!("Output size: {} x {}", settings.size[0], settings.size[1]),
                Action::CycleExportSize,
            ),
            (
                format!("Output FPS: {}", settings.fps),
                Action::CycleExportFps,
            ),
            (
                format!("Format: {} (MP4)", codec_label(settings.codec)),
                Action::CycleExportCodec,
            ),
            (
                format!("Encoder: {}", encoder_label(settings.encoder)),
                Action::CycleExportEncoder,
            ),
            (
                format!(
                    "Export audio: {}",
                    if settings.include_audio { "On" } else { "Off" }
                ),
                Action::ToggleExportAudio,
            ),
        ] {
            row(
                canvas,
                view,
                [left, y, right, bottom],
                &label,
                action,
                enabled,
            )?;
            y += canvas.r(5.0);
        }
        let button = canvas.r(5.6);
        row(
            canvas,
            view,
            [left, y, right, bottom],
            if settings.include_recorded_hud {
                "Replay HUD and chat: Enabled"
            } else {
                "Replay HUD and chat: Hidden"
            },
            Action::ToggleRecordedHud,
            enabled,
        )?;
        y += canvas.r(5.0);
        y += canvas.text("Only the recorded source camera can restore captured HUD/chat. World nametags are a separate layer. HUD/chat already baked into video pixels cannot be removed; use the native replay track.", [left, y], right-left, CAPTION, TEXT_DIMMER, false)? + canvas.r(2.0);
        canvas.text_line(
            &format!("Quality: {} (lower is higher quality)", settings.quality),
            [left, y + canvas.r(0.8)],
            (right - left - button * 2.0 - pad).max(0.0),
            BODY,
            TEXT,
        )?;
        for (index, (label, direction)) in [("-", -1), ("+", 1)].into_iter().enumerate() {
            let x = right - button * (2 - index) as f32;
            control(
                canvas,
                view,
                [x, y, x + button - canvas.r(0.4), y + canvas.r(4.0)],
                label,
                Action::AdjustExportQuality(direction),
                enabled,
                Variant::Secondary,
            )?;
        }
        y += canvas.r(5.6);
        let workflow = &view.studio.workflow;
        let path = workflow.destination.as_ref().map_or_else(
            || "Studio exports folder (automatic filenames)".to_owned(),
            |path| path.display().to_string(),
        );
        canvas.text_line(
            "Destination folder or MP4 file",
            [left, y],
            right - left,
            BODY,
            TEXT,
        )?;
        y += canvas.r(3.4);
        widgets::text_area(
            canvas,
            view,
            [left, y, right, y + canvas.r(6.0)],
            workflow.destination_draft.as_deref().unwrap_or(&path),
            "Enter an absolute destination path",
            view.field == Some(launcher::menu::MenuField::StudioDestination),
            MenuAction::Studio(Action::EditExportDestination),
        )?;
        y += canvas.r(7.0);
        if workflow.destination_draft.is_some() {
            let middle = (left + right) * 0.5;
            control(
                canvas,
                view,
                [left, y, middle - pad * 0.5, y + canvas.r(4.0)],
                "Apply destination",
                Action::ApplyExportDestination,
                enabled,
                Variant::Primary,
            )?;
            control(
                canvas,
                view,
                [middle + pad * 0.5, y, right, y + canvas.r(4.0)],
                "Cancel",
                Action::CancelExportDestination,
                true,
                Variant::Secondary,
            )?;
            y += canvas.r(5.0);
        }
        y += canvas.text("Existing files are preserved. Export renders every output frame at the chosen FPS; this cannot recreate missing recorded motion. For slow motion, use a clip speed such as 0.25x with a high FPS source.", [left, y], right - left, CAPTION, TEXT_DIMMER, false)? + canvas.r(2.0);
        row(
            canvas,
            view,
            [left, y, right, bottom],
            if view.studio.export.phase.busy() {
                "Export in progress"
            } else {
                "Export video"
            },
            Action::Export,
            enabled
                && !view.studio.export.phase.busy()
                && project.duration() > recording_studio::MediaTime::ZERO,
        )?;
        y += canvas.r(5.0);
    } else {
        y += canvas.text(
            "Open or create a project to configure output size, FPS, quality and destination.",
            [left, y],
            right - left,
            BODY,
            TEXT_DIMMER,
            false,
        )?;
    }
    Ok(y)
}

/// Native wide controls wrap the setting's value in the same focusable hit target.
fn row(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
    label: &str,
    action: Action,
    enabled: bool,
) -> Result<(), PaintError> {
    control(
        canvas,
        view,
        [area[0], area[1], area[2], area[1] + canvas.r(4.0)],
        label,
        action,
        enabled,
        Variant::Secondary,
    )
}
