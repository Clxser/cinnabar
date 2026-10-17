//! Menu text/navigation keys and editor shortcuts share one physical input stream.

use super::*;

/// Preserves edit-box priority, then routes studio commands before ordinary menu navigation.
pub(super) fn drive(
    menu: &mut MenuRuntime,
    messages: &mut bevy::prelude::MessageReader<KeyboardInput>,
    modifiers: &mut MenuModifiers,
    clipboard: &mut MenuClipboard,
    native_settings: bool,
) {
    for input in messages.read() {
        modifiers.observe(input);
        if input.state != ButtonState::Pressed {
            continue;
        }
        if !input.repeat
            && matches!(
                menu.field,
                Some(
                    MenuField::StudioDestination
                        | MenuField::StudioCamera(_)
                        | MenuField::StudioClip(_)
                )
            )
        {
            use launcher::menu::studio::Action;
            let actions = if menu.field == Some(MenuField::StudioDestination) {
                (
                    Action::ApplyExportDestination,
                    Action::CancelExportDestination,
                )
            } else if matches!(menu.field, Some(MenuField::StudioClip(_))) {
                (Action::ApplyClipTime, Action::CancelClipTime)
            } else {
                (Action::ApplyCameraValue, Action::CancelCameraValue)
            };
            let action = match input.key_code {
                KeyCode::Escape => Some(actions.1),
                KeyCode::Enter | KeyCode::NumpadEnter => Some(actions.0),
                _ => None,
            };
            if let Some(action) = action {
                menu.activate(launcher::menu::MenuAction::Studio(action));
                continue;
            }
        }
        if menu.field == Some(MenuField::StudioTitle)
            && matches!(input.key_code, KeyCode::ArrowUp | KeyCode::ArrowDown)
        {
            let direction = if input.key_code == KeyCode::ArrowUp {
                -1
            } else {
                1
            };
            menu.edit_field(|editor| editor.move_line(direction, modifiers.shift()));
            continue;
        }
        if menu.field == Some(MenuField::StudioTitle) && !input.repeat {
            let action = match input.key_code {
                KeyCode::Escape => Some(launcher::menu::studio::Action::CancelTitleText),
                KeyCode::Enter | KeyCode::NumpadEnter if !modifiers.shift() => {
                    Some(launcher::menu::studio::Action::ApplyTitleText)
                }
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    menu.edit_field(|editor| {
                        let _ = editor.insert("\n");
                    });
                    continue;
                }
                _ => None,
            };
            if let Some(action) = action {
                menu.activate(launcher::menu::MenuAction::Studio(action));
                continue;
            }
        }
        if matches!(
            input.key_code,
            KeyCode::ArrowLeft
                | KeyCode::ArrowRight
                | KeyCode::ArrowUp
                | KeyCode::ArrowDown
                | KeyCode::Tab
        ) {
            menu.input_mode = MenuInputMode::Keyboard;
        }
        if menu.edit_key(input.key_code, modifiers.shift(), modifiers.shortcut()) {
            continue;
        }
        if studio_shortcut(menu, input, modifiers) {
            continue;
        }
        if modifiers.shortcut() && menu.has_focused_field() {
            match input.key_code {
                KeyCode::KeyA => {
                    menu.select_all_text();
                    continue;
                }
                KeyCode::KeyC => {
                    if let Some(text) = menu.selected_text() {
                        clipboard.write_text(text.to_owned());
                    }
                    continue;
                }
                KeyCode::KeyV => {
                    let maximum = menu.remaining_text_capacity();
                    if let Some(text) = clipboard.read_text_bounded(maximum) {
                        menu.edit_text(&text);
                    }
                    continue;
                }
                _ => {}
            }
            if input.text.is_some() {
                continue;
            }
        }
        match input.key_code {
            KeyCode::Escape | KeyCode::Backspace | KeyCode::Enter | KeyCode::NumpadEnter
                if input.repeat => {}
            KeyCode::Space if input.repeat && !menu.has_focused_field() => {}
            KeyCode::Escape => menu.go_back_from_input(),
            KeyCode::Backspace if native_settings => menu.go_back_from_input(),
            KeyCode::ArrowUp => {
                menu.move_directional_focus(launcher::menu::view::SettingsFocusAxis::Vertical, -1)
            }
            KeyCode::ArrowLeft => menu.move_horizontal_focus(-1),
            KeyCode::ArrowDown => {
                menu.move_directional_focus(launcher::menu::view::SettingsFocusAxis::Vertical, 1)
            }
            KeyCode::ArrowRight => menu.move_horizontal_focus(1),
            KeyCode::Tab => menu.move_focus(if modifiers.shift() { -1 } else { 1 }),
            KeyCode::Enter | KeyCode::NumpadEnter if menu.field == Some(MenuField::SkinName) => {
                menu.activate(launcher::menu::MenuAction::DressingRoom(
                    launcher::dressing_room::Action::SaveRename,
                ));
            }
            KeyCode::Enter | KeyCode::NumpadEnter => menu.activate_focused(),
            KeyCode::Space if native_settings && !menu.has_focused_field() => {
                menu.activate_focused()
            }
            _ if menu.has_focused_field() && !modifiers.shortcut() => {
                if let Some(text) = input.text.as_deref() {
                    menu.edit_text(text);
                }
            }
            _ => {}
        }
    }
}

/// Editor shortcuts never steal text, modal confirmation or loading cancellation input.
fn studio_shortcut(
    menu: &mut MenuRuntime,
    input: &KeyboardInput,
    modifiers: &MenuModifiers,
) -> bool {
    use launcher::menu::{MenuScreen, studio::Action};
    if menu.screen != MenuScreen::RecordingStudio
        || menu.has_focused_field()
        || menu.dialog.is_some()
        || menu.studio.view.discard.is_some()
        || menu.studio.view.loading
    {
        return false;
    }
    let editing = !menu.studio.view.show_library
        && !menu.studio.view.workflow.show_settings
        && menu.studio.view.project.is_some();
    let timeline_focus = matches!(
        menu.focus_actions().get(menu.focused),
        Some(launcher::menu::MenuAction::Studio(
            Action::Select(_)
                | Action::DragItem { .. }
                | Action::Scrub { .. }
                | Action::TimelineOverview { .. }
                | Action::Step(_)
                | Action::Seek(_)
                | Action::Zoom(_)
                | Action::Pan(_)
                | Action::FitTimeline
                | Action::RevealPlayhead
        ))
    );
    let timeline_keys = editing && timeline_focus;
    let action = match input.key_code {
        KeyCode::KeyS if modifiers.shortcut() => Action::Save,
        KeyCode::KeyN if modifiers.shortcut() => Action::NewProject,
        KeyCode::KeyZ if modifiers.shortcut() && modifiers.shift() => Action::Redo,
        KeyCode::KeyZ if modifiers.shortcut() => Action::Undo,
        KeyCode::KeyY if modifiers.shortcut() => Action::Redo,
        KeyCode::Space if editing => Action::TogglePlayback,
        KeyCode::ArrowLeft if timeline_keys => Action::Step(-1),
        KeyCode::ArrowRight if timeline_keys => Action::Step(1),
        KeyCode::KeyS if editing && !modifiers.shortcut() => Action::Split,
        KeyCode::Delete if timeline_keys => Action::DeleteSelected,
        KeyCode::KeyF if editing && !modifiers.shortcut() => Action::FitTimeline,
        KeyCode::Equal if editing => Action::Zoom(1),
        KeyCode::Minus if editing => Action::Zoom(-1),
        KeyCode::PageUp if editing => Action::Pan(-1),
        KeyCode::PageDown if editing => Action::Pan(1),
        KeyCode::Home if timeline_keys => Action::Seek(recording_studio::MediaTime::ZERO),
        KeyCode::End if timeline_keys => Action::Seek(
            menu.studio
                .view
                .project
                .as_ref()
                .map_or(recording_studio::MediaTime::ZERO, |project| {
                    project.duration()
                }),
        ),
        _ => return false,
    };
    if !input.repeat || matches!(action, Action::Step(_)) {
        menu.input_mode = MenuInputMode::Keyboard;
        menu.studio.activate(action);
    }
    true
}
