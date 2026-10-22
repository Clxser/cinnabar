//! Selection properties have stable headings and directly accessible groups.

use super::*;
use launcher::menu::studio::{InspectorSection, WorkspaceAction};

/// Keeps creation commands out of the selection inspector and avoids nested scrolling.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), PaintError> {
    widgets::panel(canvas, area)?;
    let pad = canvas.r(1.2);
    let left = area[0] + pad;
    let right = area[2] - pad;
    let mut top = area[1] + pad;
    canvas.text_line("Properties", [left, top], right - left, HEADER5, TEXT)?;
    top += canvas.r(4.0);
    let project = view.studio.project.as_ref().expect("editor has project");
    let selected = view.studio.selected;
    let clip = project.clips.iter().find(|clip| Some(clip.id) == selected);
    if clip.is_some() {
        let sections = InspectorSection::ALL;
        let gap = canvas.r(0.4);
        let width = (right - left - gap * (sections.len() - 1) as f32) / sections.len() as f32;
        for (index, section) in sections.iter().copied().enumerate() {
            let x = left + index as f32 * (width + gap);
            control(
                canvas,
                view,
                [x, top, x + width, top + canvas.r(3.4)],
                section.label(),
                Action::Workspace(WorkspaceAction::Inspector(section)),
                true,
                if view.studio.workspace.inspector == section {
                    Variant::Primary
                } else {
                    Variant::Secondary
                },
            )?;
        }
        top += canvas.r(4.4);
    }
    let previous = canvas.settings_scrollbars;
    if right <= left || top >= area[3] - pad {
        return Ok(());
    }
    canvas.settings_scrollbars = true;
    let kind = if clip.is_some() {
        "clip"
    } else if project.shots.iter().any(|item| Some(item.id) == selected) {
        "camera"
    } else if project.effects.iter().any(|item| Some(item.id) == selected) {
        "effect"
    } else if project.titles.iter().any(|item| Some(item.id) == selected) {
        "title"
    } else if project.audio.iter().any(|item| Some(item.id) == selected) {
        "audio"
    } else {
        "empty"
    };
    let key = format!(
        "studio_properties_{}_{:?}",
        kind, view.studio.workspace.inspector
    );
    let scroll = canvas.begin_scroll(&key, [left, top, right, area[3] - pad])?;
    let y = top - scroll.offset;
    let content = [left, y, right, area[3] - pad];
    let bottom = if let Some(shot) = project.shots.iter().find(|item| Some(item.id) == selected) {
        super::super::camera::inspector(canvas, view, content, shot)?
    } else if let Some(effect) = project
        .effects
        .iter()
        .find(|item| Some(item.id) == selected)
    {
        super::super::effects::inspector(canvas, view, content, effect)?
    } else if let Some(title) = project.titles.iter().find(|item| Some(item.id) == selected) {
        super::super::titles::inspector(canvas, view, content, title)?
    } else if let Some(audio) = project.audio.iter().find(|item| Some(item.id) == selected) {
        super::super::audio::inspector(canvas, view, content, audio)?
    } else if let Some(clip) = clip {
        match view.studio.workspace.inspector {
            InspectorSection::Clip => super::super::clips::general(canvas, view, content, clip)?,
            InspectorSection::Framing => super::super::clips::framing(canvas, view, content, clip)?,
            InspectorSection::Cut => super::super::clips::cut(canvas, view, content, clip)?,
            InspectorSection::Speed => {
                let y = super::super::speed::rates(canvas, view, content, false, true)?;
                super::super::speed::draw(canvas, view, [left, y, right, content[3]], clip)?
            }
        }
    } else {
        y + canvas.text("Select a clip, camera, effect, title or audio item on the timeline to edit its properties.", [left, y], right - left, BODY, TEXT_DIMMER, false)?
    };
    let result = canvas.end_scroll(scroll, (bottom - y).max(0.0));
    canvas.settings_scrollbars = previous;
    result
}
