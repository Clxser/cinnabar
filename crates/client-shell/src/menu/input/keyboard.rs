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
                        | MenuField::StudioSeek
                        | MenuField::StudioLibrarySearch
                        | MenuField::StudioCamera(_)
                        | MenuField::StudioCameraViewName(_)
                        | MenuField::StudioClip(_)
                        | MenuField::StudioExportRange(_)
                )
            )
        {
            use launcher::menu::studio::Action;
            let actions = if menu.field == Some(MenuField::StudioDestination) {
                (
                    Action::ApplyExportDestination,
                    Action::CancelExportDestination,
                )
            } else if matches!(menu.field, Some(MenuField::StudioCameraViewName(_))) {
                use launcher::menu::studio::CameraViewsAction;
                (
                    Action::CameraViews(CameraViewsAction::Save),
                    Action::CameraViews(CameraViewsAction::Cancel),
                )
            } else if menu.field == Some(MenuField::StudioLibrarySearch) {
                use launcher::menu::studio::library::LibraryAction;
                (
                    Action::LibraryTools(LibraryAction::ApplySearch),
                    Action::LibraryTools(LibraryAction::CancelSearch),
                )
            } else if menu.field == Some(MenuField::StudioSeek) {
                use launcher::menu::studio::monitoring::MonitoringAction;
                (
                    Action::Monitoring(MonitoringAction::ApplySeek),
                    Action::Monitoring(MonitoringAction::CancelSeek),
                )
            } else if matches!(menu.field, Some(MenuField::StudioClip(_))) {
                (Action::ApplyClipTime, Action::CancelClipTime)
            } else if matches!(menu.field, Some(MenuField::StudioExportRange(_))) {
                (Action::ApplyExportRange, Action::CancelExportRange)
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
        if menu.screen() == launcher::menu::MenuScreen::RecordingStudio
            && menu.field.is_none()
            && menu.studio.view.camera_workspace.focused
            && !modifiers.shortcut()
            && matches!(
                input.key_code,
                KeyCode::KeyW
                    | KeyCode::KeyA
                    | KeyCode::KeyS
                    | KeyCode::KeyD
                    | KeyCode::KeyQ
                    | KeyCode::KeyE
            )
        {
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
        || menu.studio.view.workflow.recovery.visible
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
                | Action::Timeline(_)
        ))
    );
    let timeline_keys = editing && timeline_focus && !menu.studio.view.camera_workspace.focused;
    use launcher::menu::studio::shortcuts::{self, Context};
    if modifiers.alt() {
        return false;
    }
    let Some(key) = shortcuts::Key::ALL
        .iter()
        .copied()
        .find(|&key| studio_physical_key(key) == input.key_code)
    else {
        return false;
    };
    let Some(command) = shortcuts::resolve(
        key,
        modifiers.shortcut(),
        modifiers.shift(),
        Context {
            project: menu.studio.view.project.is_some(),
            editor: editing,
            timeline: timeline_keys,
            camera_focused: menu.studio.view.camera_workspace.focused,
            inspected: menu.studio.view.camera_workspace.view.is_some(),
        },
    ) else {
        return false;
    };
    let duration = menu
        .studio
        .view
        .project
        .as_ref()
        .map_or(recording_studio::MediaTime::ZERO, |project| {
            project.duration()
        });
    let action = command.action(duration);
    if !input.repeat || matches!(action, Action::Step(_)) {
        menu.input_mode = MenuInputMode::Keyboard;
        if matches!(
            action,
            Action::Monitoring(launcher::menu::studio::monitoring::MonitoringAction::EditSeek)
        ) {
            menu.activate(launcher::menu::MenuAction::Studio(action));
        } else {
            menu.studio.activate(action);
        }
    }
    true
}

/// Exhaustive conversion keeps the engine edge aligned with the portable help/command registry.
fn studio_physical_key(key: launcher::menu::studio::shortcuts::Key) -> KeyCode {
    use launcher::menu::studio::shortcuts::Key;
    match key {
        Key::S => KeyCode::KeyS,
        Key::N => KeyCode::KeyN,
        Key::Z => KeyCode::KeyZ,
        Key::Y => KeyCode::KeyY,
        Key::E => KeyCode::KeyE,
        Key::R => KeyCode::KeyR,
        Key::A => KeyCode::KeyA,
        Key::D => KeyCode::KeyD,
        Key::F => KeyCode::KeyF,
        Key::G => KeyCode::KeyG,
        Key::L => KeyCode::KeyL,
        Key::BracketLeft => KeyCode::BracketLeft,
        Key::BracketRight => KeyCode::BracketRight,
        Key::Space => KeyCode::Space,
        Key::Left => KeyCode::ArrowLeft,
        Key::Right => KeyCode::ArrowRight,
        Key::Delete => KeyCode::Delete,
        Key::Plus => KeyCode::Equal,
        Key::Minus => KeyCode::Minus,
        Key::PageUp => KeyCode::PageUp,
        Key::PageDown => KeyCode::PageDown,
        Key::Home => KeyCode::Home,
        Key::End => KeyCode::End,
        Key::Help => KeyCode::F1,
    }
}

#[cfg(test)]
mod studio_shortcut_tests {
    use super::*;
    #[test]
    fn registered_portable_keys_have_unique_physical_inputs() {
        let keys = launcher::menu::studio::shortcuts::Key::ALL;
        for (index, &key) in keys.iter().enumerate() {
            assert!(
                !keys[index + 1..]
                    .iter()
                    .any(|&other| studio_physical_key(key) == studio_physical_key(other))
            );
        }
    }
}
