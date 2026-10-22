use launcher::menu::settings_options::SettingId;
mod cancel;
pub(super) mod clipboard;
pub(super) mod command_macros;
pub mod context;
mod death;
mod modifiers;
use modifiers::MenuModifiers;
pub(super) mod fields;
mod ime;
mod keyboard;
pub use ime::drive_menu_ime;
mod preview;
mod remapping;
mod scroll;
mod server_list;
mod settings_pointer;
mod studio_camera;

use clipboard::MenuClipboard;
use context::MenuInputContext;
use settings_pointer::{native_release_action, update_slider};

use bevy::{
    ecs::message::{MessageCursor, Messages},
    input::{
        ButtonState,
        gamepad::{Gamepad, GamepadButton},
        keyboard::KeyboardInput,
        mouse::{MouseButtonInput, MouseScrollUnit, MouseWheel},
        touch::Touches,
    },
    prelude::{ButtonInput, Entity, KeyCode, Local, MouseButton, Query, Res, ResMut, Single, With},
    window::{CursorGrabMode, CursorOptions, PrimaryWindow, Window},
};
use ui::{ChatEditor, ToastPress, UiPoint};

use crate::ui_resources::{UiPresentationResource, UiRuntimeResource};
use client_ui::ui_runtime::presentation::UiPresentationRuntime;
use {super::MenuRuntime, launcher::menu::MenuField};

/// Vanilla's fixed desktop option hotkeys.
pub const HOTKEY_OPTIONS: [(KeyCode, SettingId); 2] = [
    (KeyCode::F1, SettingId::HideHud),
    (KeyCode::F8, SettingId::HidePaperdoll),
];

#[derive(Clone, Copy, Debug, Default)]
pub(super) enum MenuInputMode {
    #[default]
    Mouse,
    Touch,
    Keyboard,
    Gamepad,
}

impl MenuInputMode {
    pub(super) fn mouse(self) -> bool {
        matches!(self, Self::Mouse)
    }

    pub(super) fn navigation(self) -> bool {
        matches!(self, Self::Keyboard | Self::Gamepad)
    }

    pub(super) fn gamepad(self) -> bool {
        matches!(self, Self::Gamepad)
    }
}

/// Retains physical button state because menu input clears Bevy's buttons
/// after consumption, and retains slider capture across UI scale relayout.
#[derive(Default)]
pub struct GuiScaleDrag {
    mouse_cursor: MessageCursor<MouseButtonInput>,
    left_held: bool,
    captured: bool,
    previous_pointer: Option<bevy::prelude::Vec2>,
    pointer_anchor: Option<bevy::prelude::Vec2>,
    touch_slider: Option<(u64, u16)>,
    pending_press: Option<launcher::menu::MenuAction>,
    touch_press: Option<(u64, launcher::menu::MenuAction)>,
    preview_touch: Option<u64>,
    server_list_touch: Option<u64>,
    scroll_touch: Option<u64>,
}

impl MenuRuntime {
    /// A press inside the focused field puts its caret at `byte`.
    pub fn place_caret(&mut self, byte: usize) {
        self.edit_field(|editor| editor.place_cursor(byte));
    }

    fn select_all_text(&mut self) {
        self.edit_field(|editor| {
            editor.move_home(false);
            editor.move_end(true);
        });
    }

    fn selected_text(&self) -> Option<&str> {
        let editor = self.editor(self.field?);
        editor.selection().map(|range| &editor.as_str()[range])
    }

    fn remaining_text_capacity(&self) -> usize {
        self.field
            .map_or(0, |field| self.editor(field).remaining_insert_capacity())
    }

    /// Insert what fits of `text` at the caret, replacing any selection.
    pub(super) fn edit_text(&mut self, text: &str) {
        let Some(field) = self.field else {
            return;
        };
        // The port box takes number characters only, as vanilla's `NumberChars` text type.
        let accepts = |character: &char| field != MenuField::Port || character.is_ascii_digit();
        self.edit_field(|editor| {
            let capacity = editor.remaining_insert_capacity();
            let mut insertion = String::new();
            for character in text
                .chars()
                .filter(|character| {
                    !character.is_control() || field == MenuField::StudioTitle && *character == '\n'
                })
                .filter(accepts)
            {
                if insertion.len() + character.len_utf8() > capacity {
                    break;
                }
                insertion.push(character);
            }
            // Within the remaining capacity, so the insert cannot be refused; nothing
            // accepted leaves a selection standing.
            if !insertion.is_empty() {
                let _ = editor.insert(&insertion);
            }
        });
    }

    /// A caret or deletion key in the focused field, acting on whole words when `by_word`;
    /// `false` leaves it to focus navigation.
    fn edit_key(&mut self, key: KeyCode, selecting: bool, by_word: bool) -> bool {
        let edit: fn(&mut ChatEditor, bool) = match key {
            KeyCode::ArrowLeft if by_word => ChatEditor::move_word_left,
            KeyCode::ArrowRight if by_word => ChatEditor::move_word_right,
            KeyCode::Backspace if by_word => |editor, _| editor.backspace_word(),
            KeyCode::Delete if by_word => |editor, _| editor.delete_word(),
            KeyCode::ArrowLeft => |editor, selecting| {
                if selecting {
                    editor.select_left();
                } else {
                    editor.move_left();
                }
            },
            KeyCode::ArrowRight => |editor, selecting| {
                if selecting {
                    editor.select_right();
                } else {
                    editor.move_right();
                }
            },
            KeyCode::Home => ChatEditor::move_home,
            KeyCode::End => ChatEditor::move_end,
            KeyCode::Backspace => |editor, _| editor.backspace(),
            KeyCode::Delete => |editor, _| editor.delete(),
            _ => return false,
        };
        if !self.has_focused_field() {
            return false;
        }
        self.edit_field(|editor| edit(editor, selecting));
        true
    }
}

#[allow(clippy::too_many_arguments)]
pub fn drive_menu_input(
    context: MenuInputContext,
    wheel_messages: Option<Res<Messages<MouseWheel>>>,
    mut wheel_cursor: Local<MessageCursor<MouseWheel>>,
    window: Single<(Entity, &Window, &mut CursorOptions), With<PrimaryWindow>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse_buttons: ResMut<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    gamepads: Query<&Gamepad>,
    mut presentation: ResMut<UiPresentationResource>,
    mut clipboard: ResMut<MenuClipboard>,
    mut menu: ResMut<MenuRuntime>,
    runtime: Option<Res<UiRuntimeResource>>,
    mut modifiers: Local<MenuModifiers>,
    consent: Option<Res<client_state::ConsentInput>>,
    mouse_messages: Option<Res<Messages<MouseButtonInput>>>,
    mut gui_scale_drag: Local<GuiScaleDrag>,
) {
    let MenuInputContext {
        player_runtime,
        time,
        mut keyboard_messages,
        mut focus,
        driven,
        keyboard_events,
        motion_events,
        mut camera_input,
    } = context;
    let elapsed = time.as_ref().map_or(0.0, |time| time.delta_secs_f64());
    camera_input.observe(
        keyboard_events.as_deref(),
        mouse_messages.as_deref(),
        motion_events.as_deref(),
        menu.studio.view.camera_workspace.focused,
        window.1.focused,
    );
    if driven.is_none()
        && (!window.1.focused || focus.as_ref().is_some_and(|focus| !focus.available()))
        || !menu.is_visible()
        || consent.as_ref().is_some_and(|consent| consent.0)
        || runtime.as_ref().is_some_and(|runtime| {
            runtime.server_forms().owns_input() || runtime.credits().owns_input()
        })
    {
        scroll::drive(
            &mut presentation,
            &touches,
            false,
            &mut gui_scale_drag.scroll_touch,
        );
    }
    if let Some(time) = time {
        menu.advance_death_controls(time.delta_secs_f64());
        let studio_visible =
            menu.is_visible() && menu.screen() == launcher::menu::MenuScreen::RecordingStudio;
        menu.studio.tick(time.delta_secs_f64(), studio_visible);
    }
    menu.settings.slider_hovered = None;
    if consent.is_some_and(|consent| consent.0) {
        menu.studio.cancel_pointer();
        presentation.cancel_menu_player_preview_input();
        presentation.cancel_menu_server_list_input();
        gui_scale_drag.preview_touch = None;
        menu.settings.slider_pointer = None;
        menu.settings_input.slider_drag = None;
        menu.settings_input.shader_slider_drag = None;
        gui_scale_drag.pending_press = None;
        gui_scale_drag.touch_press = None;
        gui_scale_drag.touch_slider = None;
        keyboard_messages.clear();
        menu.pressed = None;
        menu.hovered = None;
        menu.pointer_down = false;
        *modifiers = MenuModifiers::default();
        gui_scale_drag.captured = false;
        gui_scale_drag.left_held = false;
        if let Some(messages) = mouse_messages.as_deref() {
            gui_scale_drag.mouse_cursor.clear(messages);
        }
        if let Some(messages) = wheel_messages.as_deref() {
            wheel_cursor.clear(messages);
        }
        return;
    }
    menu.refresh_presented_focus(&presentation);
    let (window_entity, window, mut cursor) = window.into_inner();
    if let Some(messages) = mouse_messages.as_deref() {
        let GuiScaleDrag {
            mouse_cursor,
            left_held,
            ..
        } = &mut *gui_scale_drag;
        for input in mouse_cursor.read(messages) {
            if input.window == window_entity && input.button == MouseButton::Left {
                *left_held = input.state == ButtonState::Pressed;
            }
        }
    }
    let wheel: Vec<([f32; 2], bool)> = wheel_messages
        .as_deref()
        .map(|messages| {
            wheel_cursor
                .read(messages)
                .map(|wheel| ([wheel.x, wheel.y], wheel.unit == MouseScrollUnit::Pixel))
                .collect()
        })
        .unwrap_or_default();
    if gui_scale_drag.observe_death(
        runtime.as_deref().map(|runtime| &runtime.0),
        &mut menu,
        &mut presentation,
        &mut keyboard_messages,
    ) {
        *modifiers = MenuModifiers::default();
        keys.reset_all();
        mouse_buttons.reset_all();
        crate::physical_input::release_cursor(&mut cursor);
        return;
    }
    if runtime.as_ref().is_some_and(|runtime| {
        runtime.credits().owns_input()
            || runtime.server_forms().owns_input()
                && (!menu.is_visible() || runtime.server_forms().settings_form_active())
    }) {
        menu.studio.release_camera();
        presentation.cancel_menu_player_preview_input();
        presentation.cancel_menu_server_list_input();
        gui_scale_drag.preview_touch = None;
        menu.settings.slider_pointer = None;
        menu.settings_input.slider_drag = None;
        menu.settings_input.shader_slider_drag = None;
        gui_scale_drag.pending_press = None;
        gui_scale_drag.touch_press = None;
        gui_scale_drag.touch_slider = None;
        gui_scale_drag.captured = false;
        gui_scale_drag.left_held = false;
        keyboard_messages.clear();
        return;
    }
    menu.pressed = None;
    if driven.is_none()
        && (!window.focused || focus.as_ref().is_some_and(|focus| !focus.available()))
    {
        menu.studio.cancel_pointer();
        presentation.cancel_menu_player_preview_input();
        presentation.cancel_menu_server_list_input();
        gui_scale_drag.preview_touch = None;
        menu.settings.slider_pointer = None;
        menu.settings_input.slider_drag = None;
        menu.settings_input.shader_slider_drag = None;
        gui_scale_drag.pending_press = None;
        gui_scale_drag.touch_press = None;
        gui_scale_drag.touch_slider = None;
        gui_scale_drag.captured = false;
        gui_scale_drag.left_held = false;
        if !menu.is_visible()
            && menu.settings_options.value(SettingId::PauseMenuOnFocusLost) != 0
            && !crate::screen_policy::absorbs_input(
                &player_runtime,
                runtime.as_deref().map(|runtime| &runtime.0),
                Some(&menu),
                Some(&presentation),
            )
        {
            menu.open_pause();
            crate::physical_input::release_cursor(&mut cursor);
        }
        *modifiers = MenuModifiers::default();
        keyboard_messages.clear();
        menu.pointer_down = false;
        return;
    }
    if !menu.is_visible()
        && runtime
            .as_ref()
            .is_none_or(|runtime| !runtime.ui_focused(&player_runtime))
    {
        // Vanilla desktop input uses fixed F1/F8 shortcuts.
        for (key, option) in HOTKEY_OPTIONS {
            if keys.just_pressed(key) {
                let value = 1 - menu.settings_options.value(option);
                if menu.transient_toggles {
                    menu.set_session_option(option, Some(value));
                } else {
                    menu.set_named_option(option, value);
                }
            }
        }
    }
    if !menu.is_visible() {
        menu.studio.cancel_pointer();
        presentation.cancel_menu_player_preview_input();
        presentation.cancel_menu_server_list_input();
        gui_scale_drag.preview_touch = None;
        menu.settings.slider_pointer = None;
        menu.settings_input.slider_drag = None;
        menu.settings_input.shader_slider_drag = None;
        gui_scale_drag.pending_press = None;
        gui_scale_drag.touch_press = None;
        gui_scale_drag.touch_slider = None;
        gui_scale_drag.captured = false;
        // Gameplay/chat handled these messages already. In particular, do not
        // replay the Escape that opens pause as "back" on the following frame.
        *modifiers = MenuModifiers::default();
        // An Escape held since a screen closed reaches `keys` as a fresh press on each OS repeat.
        let (mut escape_pressed, mut escape_fresh) = (false, false);
        for input in keyboard_messages.read() {
            if input.key_code == KeyCode::Escape && input.state == ButtonState::Pressed {
                escape_pressed = true;
                escape_fresh |= !input.repeat;
            }
        }
        menu.hovered = None;
        menu.pointer_down = false;
        if keys.just_pressed(KeyCode::Escape) && (escape_fresh || !escape_pressed) {
            modifiers.capture_pressed(&keys);
            menu.open_pause();
            crate::physical_input::release_cursor(&mut cursor);
            keys.reset_all();
        } else if cursor.grab_mode == CursorGrabMode::None
            && mouse_buttons.just_pressed(MouseButton::Left)
            && window
                .cursor_position()
                .and_then(|position| UiPoint::new(position.x, position.y).ok())
                .and_then(|point| presentation.toast_press_at(point))
                == Some(ToastPress::JoinRequests)
        {
            // A free cursor pressing the join request toast opens its popup, and nothing beneath;
            // the held press is not a new one for the menu.
            menu.open_join_requests();
            menu.pointer_down = true;
            mouse_buttons.reset(MouseButton::Left);
        }
        return;
    }

    if menu.settings.key_remap.is_some() {
        remapping::capture(
            &mut menu,
            &mut keyboard_messages,
            &mut keys,
            &mut mouse_buttons,
            &gamepads,
        );
        return;
    }

    if mouse_buttons.just_pressed(MouseButton::Back) {
        menu.go_back_from_input();
        gui_scale_drag.cancel_pointer(&mut menu, &mut presentation);
        modifiers.capture_pressed(&keys);
        for input in keyboard_messages.read() {
            modifiers.observe(input);
        }
        keys.reset_all();
        mouse_buttons.reset_all();
        if !menu.is_visible()
            && !menu.intents.disconnect
            && let Some(focus) = focus.as_deref_mut()
        {
            focus.authorize_screen_return();
        }
        return;
    }

    let gameplay_pending = menu.gameplay_return_pending();
    let respawn_pending = menu.death_loading;
    modifiers.capture_pressed(&keys);
    crate::physical_input::release_cursor(&mut cursor);
    // The join request popup takes the keyboard from any box focused beneath it.
    if menu.join_request_prompted() {
        menu.field = None;
    }
    let native_settings = presentation.uses_oreui_settings();
    let pointer = window
        .cursor_position()
        .and_then(|position| UiPoint::new(position.x, position.y).ok());
    let cursor_position = window.cursor_position();
    let pointer_moved = cursor_position != gui_scale_drag.previous_pointer;
    gui_scale_drag.previous_pointer = cursor_position;
    menu.hovered = pointer.and_then(|position| presentation.hit_test_menu(position));
    menu.settings.slider_hovered = native_settings
        .then(|| pointer.and_then(|point| presentation.settings_slider_thumb_hit_test(point)))
        .flatten();
    let slider_hovered = menu.settings.slider_hovered;
    let pointer_can_focus = |action| {
        !native_settings
            || match action {
                launcher::menu::MenuAction::SettingsOption(index, _)
                    if launcher::menu::settings_options::SETTINGS_OPTIONS
                        .get(usize::from(index))
                        .is_some_and(|option| {
                            matches!(
                                option.kind,
                                launcher::menu::settings_options::SettingKind::Slider
                            )
                        }) =>
                {
                    slider_hovered == Some(index)
                }
                _ => true,
            }
    };
    let pointer_pressed = mouse_buttons.pressed(MouseButton::Left) || gui_scale_drag.left_held;
    let pointer_just_pressed =
        mouse_buttons.just_pressed(MouseButton::Left) || (pointer_pressed && !menu.pointer_down);
    let camera_owns_pointer = camera_input.drive(
        &mut menu,
        cursor_position.map(|point| point.to_array()),
        pointer_pressed,
        pointer_just_pressed,
        elapsed,
        studio_camera::viewport_bounds(&presentation.0),
    );
    let studio_owns_pointer = camera_owns_pointer
        || if menu.screen() == launcher::menu::MenuScreen::RecordingStudio {
            let hovered = menu.hovered;
            menu.studio.pointer(
                hovered,
                cursor_position.map(|position| [position.x, position.y]),
                pointer_pressed,
                pointer_just_pressed,
                [modifiers.shift(), modifiers.shortcut(), modifiers.alt()],
            )
        } else {
            false
        };
    if studio_owns_pointer {
        menu.field = None;
    }
    let server_list_owns_pointer = studio_owns_pointer
        || server_list::drive(
            &mut presentation,
            &mut menu,
            pointer,
            pointer_pressed,
            pointer_just_pressed,
            &touches,
            &mut gui_scale_drag.server_list_touch,
        );
    let preview_owns_pointer =
        if menu.dialog.is_some() || (menu.is_connecting() && menu.feeds.server_trust.is_some()) {
            gui_scale_drag.preview_touch = None;
            presentation.menu_player_preview_pointer(Some(menu.screen()), pointer, false, false);
            false
        } else {
            preview::drive(
                &mut presentation,
                menu.screen(),
                pointer,
                pointer_pressed,
                pointer_just_pressed,
                &touches,
                &mut gui_scale_drag.preview_touch,
            )
        };
    if preview_owns_pointer {
        menu.hovered = None;
    }
    let pointer_switch = match (gui_scale_drag.pointer_anchor, cursor_position) {
        (Some(anchor), Some(position)) => anchor.distance_squared(position) > 100.0,
        _ => false,
    };
    if gui_scale_drag.pointer_anchor.is_none() || pointer_switch {
        gui_scale_drag.pointer_anchor = cursor_position;
    }
    if pointer_switch || pointer_just_pressed || !wheel.is_empty() {
        menu.input_mode = MenuInputMode::Mouse;
    }
    if (pointer_moved || pointer_just_pressed)
        && let Some(action) = menu.hovered.filter(|action| pointer_can_focus(*action))
    {
        menu.focus_pointer(action);
    }
    if native_settings
        && let Some(point) = pointer
        && let Some(index) = presentation.settings_slider_thumb_hit_test(point)
    {
        menu.hovered = Some(launcher::menu::MenuAction::SettingsOption(
            index,
            menu.settings_options.get(usize::from(index)),
        ));
    }
    menu.pointer_down = pointer_pressed;
    if !pointer_pressed || menu.screen() != launcher::menu::MenuScreen::Settings {
        gui_scale_drag.captured = false;
    }
    if let Some(point) = pointer {
        for ([horizontal, vertical], pixels) in wheel {
            let studio = menu.screen() == launcher::menu::MenuScreen::RecordingStudio;
            let ruler = matches!(
                menu.hovered,
                Some(launcher::menu::MenuAction::Studio(
                    launcher::menu::studio::Action::Scrub { .. }
                        | launcher::menu::studio::Action::TimelineOverview { .. }
                ))
            );
            let hovered = menu.hovered;
            let handled = studio
                && (menu.studio.graph_wheel(
                    point,
                    hovered,
                    horizontal,
                    vertical,
                    pixels,
                    modifiers.shift(),
                    modifiers.shortcut(),
                ) || camera_input.wheel(&mut menu, vertical, pixels, modifiers.shortcut())
                    || menu.studio.timeline_wheel(
                        point,
                        presentation
                            .menu_scroll_viewport(launcher::menu::studio::TIMELINE_SCROLL_ID),
                        ruler,
                        horizontal,
                        vertical,
                        pixels,
                        modifiers.shift(),
                        modifiers.shortcut(),
                    ));
            if !handled {
                presentation.scroll_menu(point, vertical, pixels);
            }
        }
    }
    // A scrollbar press or drag scrolls instead of pressing what lies beneath.
    let on_scrollbar = server_list_owns_pointer
        || preview_owns_pointer
        || presentation.drag_menu_scroll(pointer, pointer_pressed)
        || (pointer_just_pressed
            && pointer.is_some_and(|point| presentation.press_menu_scrollbar(point)));
    if on_scrollbar {
        menu.hovered = None;
    }
    let scrolled = scroll::drive(
        &mut presentation,
        &touches,
        native_settings && menu.dialog.is_none(),
        &mut gui_scale_drag.scroll_touch,
    );
    if scrolled.is_some() {
        gui_scale_drag.touch_press = None;
        menu.pressed = None;
    }
    let press = |menu: &mut MenuRuntime, action| {
        let action = if native_settings {
            menu.live_settings_action(action)
        } else {
            action
        };
        if let Some(sound) = presentation.menu_sound(action) {
            client_ui::sound_requests::ui_control_sound(sound);
        }
        menu.activate_from_input(action);
    };
    update_slider(
        &mut menu,
        &presentation,
        pointer,
        pointer_pressed,
        pointer_just_pressed,
        native_settings,
    );
    if pointer_just_pressed
        && !on_scrollbar
        && matches!(
            menu.hovered,
            Some(launcher::menu::MenuAction::SettingsScale(_))
        )
        && pointer.is_some_and(|point| presentation.gui_scale_drag_action(point).is_some())
    {
        gui_scale_drag.captured = pointer_pressed;
    }
    if gui_scale_drag.captured
        && pointer_pressed
        && let Some(action @ launcher::menu::MenuAction::SettingsScale(offset)) =
            pointer.and_then(|point| presentation.gui_scale_drag_action(point))
    {
        menu.hovered = Some(action);
        if pointer_just_pressed || offset != menu.gui_scale_offset() {
            menu.activate(action);
        } else {
            menu.pressed = Some(action);
        }
    }
    // A press inside a text box also puts its caret at the nearest character.
    let mut caret_press = None;
    if pointer_just_pressed
        && !on_scrollbar
        && !gui_scale_drag.captured
        && let Some(action) = menu.hovered
    {
        if native_settings {
            gui_scale_drag.pending_press = (menu.settings_input.slider_drag.is_none()
                && menu.settings_input.shader_slider_drag.is_none())
            .then_some(action);
            menu.pressed = Some(action);
            if pointer_can_focus(action) {
                menu.focus_pointer(action);
            }
        } else {
            press(&mut menu, action);
            caret_press = pointer.zip(action.text_field());
        }
    }
    if pointer_pressed {
        if let Some(action) = gui_scale_drag.pending_press
            && native_release_action(action, menu.hovered).is_some()
        {
            menu.pressed = Some(action);
        }
    } else if let Some(action) = gui_scale_drag.pending_press.take()
        && native_settings
        && let Some(action) = native_release_action(action, menu.hovered)
    {
        press(&mut menu, action);
        caret_press = pointer.zip(action.text_field());
    }
    if touches.any_just_pressed() {
        menu.touch_available = true;
        menu.input_mode = MenuInputMode::Touch;
    }
    for touch in touches.iter_just_pressed() {
        if server_list_owns_pointer
            || gui_scale_drag.preview_touch == Some(touch.id())
            || scrolled == Some(touch.id())
        {
            continue;
        }
        let position = touch.position();
        if let Ok(position) = UiPoint::new(position.x, position.y)
            && let Some(action) = native_settings
                .then(|| presentation.settings_slider_thumb_hit_test(position))
                .flatten()
                .map(|index| {
                    launcher::menu::MenuAction::SettingsOption(
                        index,
                        menu.settings_options.get(usize::from(index)),
                    )
                })
                .or_else(|| presentation.hit_test_menu(position))
        {
            menu.input_mode = MenuInputMode::Touch;
            if native_settings {
                if !matches!(action, launcher::menu::MenuAction::SettingsOption(index, _)
                    if launcher::menu::settings_options::SETTINGS_OPTIONS.get(usize::from(index))
                        .is_some_and(|option| matches!(option.kind, launcher::menu::settings_options::SettingKind::Slider)))
                    || matches!(action, launcher::menu::MenuAction::SettingsOption(index, _)
                        if presentation.settings_slider_thumb_contains(index, position))
                {
                    menu.focus_pointer(action);
                }
                menu.pressed = Some(action);
                if let launcher::menu::MenuAction::SettingsOption(index, _) = action
                    && presentation.settings_slider_thumb_contains(index, position)
                {
                    gui_scale_drag.touch_slider = Some((touch.id(), index));
                    gui_scale_drag.touch_press = None;
                } else {
                    gui_scale_drag.touch_press = Some((touch.id(), action));
                }
            } else {
                press(&mut menu, action);
                caret_press = action.text_field().map(|field| (position, field));
            }
        }
    }
    if let Some((id, index)) = gui_scale_drag.touch_slider {
        if native_settings && let Some(touch) = touches.get_pressed(id) {
            let position = touch.position();
            if let Ok(point) = UiPoint::new(position.x, position.y)
                && let Some(action @ launcher::menu::MenuAction::SettingsOption(_, value)) =
                    presentation.settings_slider_drag_action(index, point)
                && let Some(fraction) = presentation.settings_slider_drag_fraction(index, point)
            {
                menu.input_mode = MenuInputMode::Touch;
                menu.set_option(index, value);
                menu.settings.slider_pointer = Some(launcher::menu::view::SettingsSliderPointer {
                    option: index,
                    fraction,
                    mouse_input: false,
                });
                menu.pressed = Some(action);
            }
        } else {
            gui_scale_drag.touch_slider = None;
            menu.settings.slider_pointer = None;
        }
    }
    if let Some((id, mut action)) = gui_scale_drag.touch_press {
        if let Some(touch) = touches.get_released(id) {
            let position = touch.position();
            let hovered = UiPoint::new(position.x, position.y)
                .ok()
                .and_then(|point| presentation.hit_test_menu(point));
            if native_settings
                && !UiPoint::new(position.x, position.y)
                    .ok()
                    .is_some_and(|point| {
                        settings_pointer::shader_touch_drag(
                            &mut menu,
                            &presentation,
                            &mut action,
                            point,
                            true,
                        )
                    })
                && let Some(action) = native_release_action(action, hovered)
            {
                press(&mut menu, action);
                caret_press = UiPoint::new(position.x, position.y)
                    .ok()
                    .zip(action.text_field());
            }
            gui_scale_drag.touch_press = None;
        } else if native_settings && let Some(touch) = touches.get_pressed(id) {
            let position = touch.position();
            let hovered = UiPoint::new(position.x, position.y)
                .ok()
                .and_then(|point| presentation.hit_test_menu(point));
            let shader_dragged = UiPoint::new(position.x, position.y)
                .ok()
                .is_some_and(|point| {
                    settings_pointer::shader_touch_drag(
                        &mut menu,
                        &presentation,
                        &mut action,
                        point,
                        false,
                    )
                });
            if shader_dragged {
                gui_scale_drag.touch_press = Some((id, action));
            } else if native_release_action(action, hovered).is_some() {
                menu.pressed = Some(action);
            }
        } else {
            gui_scale_drag.touch_press = None;
        }
    }
    if let Some((point, field)) = caret_press
        && menu.field == Some(field)
        && let Some(byte) = presentation.menu_caret_at(point, field, menu.field_text(field))
    {
        menu.place_caret(byte);
    }
    for gamepad in &gamepads {
        if gamepad.get_just_pressed().next().is_some() {
            menu.input_mode = MenuInputMode::Gamepad;
        }
        if gamepad.just_pressed(GamepadButton::DPadLeft) {
            menu.move_horizontal_focus(-1);
        }
        if gamepad.just_pressed(GamepadButton::DPadRight) {
            menu.move_horizontal_focus(1);
        }
        if gamepad.just_pressed(GamepadButton::DPadUp) {
            menu.move_directional_focus(launcher::menu::view::SettingsFocusAxis::Vertical, -1);
        }
        if gamepad.just_pressed(GamepadButton::DPadDown) {
            menu.move_directional_focus(launcher::menu::view::SettingsFocusAxis::Vertical, 1);
        }
        if gamepad.just_pressed(
            crate::menu::settings_options::control_bindings::gamepad_button(
                &menu.settings_options,
                GamepadButton::South,
            ),
        ) {
            menu.activate_focused();
        }
        if gamepad.just_pressed(
            crate::menu::settings_options::control_bindings::gamepad_button(
                &menu.settings_options,
                GamepadButton::East,
            ),
        ) {
            menu.go_back_from_input();
        }
    }
    keyboard::drive(
        &mut menu,
        &mut keyboard_messages,
        &mut modifiers,
        &mut clipboard,
        native_settings,
    );
    if (!menu.is_visible()
        || (!gameplay_pending && menu.gameplay_return_pending())
        || (!respawn_pending && menu.death_loading)
        || menu.pressed == Some(launcher::menu::MenuAction::ServerTrust(true)))
        && !menu.intents.disconnect
        && let Some(focus) = focus.as_deref_mut()
    {
        focus.authorize_screen_return();
    }
    // The menu owns the pointer and keyboard for this frame. This also keeps
    // the camera's recapture-on-click path from turning a menu click into a
    // gameplay attack or mouse grab.
    let mouse_input = menu.input_mode.mouse();
    if let Some(pointer) = menu.settings.slider_pointer.as_mut() {
        pointer.mouse_input = mouse_input;
    }
    keys.reset_all();
    mouse_buttons.reset_all();
}

#[cfg(test)]
mod gui_scale_drag_tests;

#[cfg(test)]
mod settings_slider_tests;

#[cfg(test)]
mod sign_in_tests;

#[cfg(test)]
mod death_tests;
