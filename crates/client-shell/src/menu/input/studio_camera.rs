//! Physical key state is retained independently of the menu's per-frame ButtonInput resets.

use super::*;
use bevy::input::mouse::MouseMotion;

const FORWARD: u16 = 1 << 0;
const BACKWARD: u16 = 1 << 1;
const LEFT: u16 = 1 << 2;
const RIGHT: u16 = 1 << 3;
const DOWN: u16 = 1 << 4;
const UP: u16 = 1 << 5;
const SHIFT_LEFT: u16 = 1 << 6;
const SHIFT_RIGHT: u16 = 1 << 7;
const CTRL_LEFT: u16 = 1 << 8;
const CTRL_RIGHT: u16 = 1 << 9;
const SUPER_LEFT: u16 = 1 << 10;
const SUPER_RIGHT: u16 = 1 << 11;
const ALT_LEFT: u16 = 1 << 12;
const ALT_RIGHT: u16 = 1 << 13;
const SHIFT: u16 = SHIFT_LEFT | SHIFT_RIGHT;
const ALT: u16 = ALT_LEFT | ALT_RIGHT;
const SHORTCUT: u16 = CTRL_LEFT | CTRL_RIGHT | SUPER_LEFT | SUPER_RIGHT | ALT;
const MODIFIERS: u16 = SHIFT | SHORTCUT;

#[derive(Default)]
pub struct Input {
    keyboard: MessageCursor<KeyboardInput>,
    mouse: MessageCursor<MouseButtonInput>,
    motion: MessageCursor<MouseMotion>,
    held: u16,
    right: bool,
    right_pressed: bool,
    look: [f64; 2],
    looking: bool,
    focused: bool,
}

impl Input {
    /// Every event reader advances even outside the viewport; old OS repeats cannot move a new focus.
    pub(super) fn observe(
        &mut self,
        keys: Option<&Messages<KeyboardInput>>,
        mouse: Option<&Messages<MouseButtonInput>>,
        motion: Option<&Messages<MouseMotion>>,
        focused: bool,
        available: bool,
    ) {
        if !focused {
            self.held &= MODIFIERS;
            self.looking = false;
            self.focused = false;
        }
        if !available {
            self.held = 0;
            self.right = false;
        }
        if let Some(keys) = keys {
            for input in self.keyboard.read(keys) {
                let mask = mask(input.key_code);
                if available
                    && (focused || mask & MODIFIERS != 0)
                    && input.state == ButtonState::Pressed
                {
                    self.held |= mask;
                } else {
                    self.held &= !mask;
                }
            }
        }
        self.right_pressed = false;
        if let Some(mouse) = mouse {
            for input in self.mouse.read(mouse) {
                if input.button == MouseButton::Right {
                    self.right = available && input.state == ButtonState::Pressed;
                    self.right_pressed |= self.right;
                }
            }
        }
        self.look = [0.0; 2];
        if let Some(motion) = motion {
            for input in self.motion.read(motion) {
                self.look[0] += f64::from(input.delta.x) * 0.18;
                self.look[1] += f64::from(input.delta.y) * 0.18;
            }
        }
    }

    /// The viewport owns pointer gestures before timeline and ordinary menu activation see them.
    pub(super) fn drive(
        &mut self,
        menu: &mut MenuRuntime,
        point: Option<[f32; 2]>,
        held: bool,
        pressed: bool,
        elapsed: f64,
        viewport_visible: bool,
    ) -> bool {
        use launcher::menu::{MenuAction, MenuScreen, studio::Action};
        if !viewport_visible
            || menu.screen() != MenuScreen::RecordingStudio
            || menu.dialog.is_some()
            || menu.studio.view.show_library
            || menu.studio.view.workflow.show_settings
            || menu.studio.view.discard.is_some()
            || menu.studio.view.loading
            || menu.join_request_prompted()
        {
            menu.studio.release_camera();
            return false;
        }
        let hovered = menu.hovered;
        let over_view = matches!(
            hovered,
            Some(MenuAction::Studio(
                Action::CameraViewport { .. }
                    | Action::CameraControl { .. }
                    | Action::CameraSubject(_)
            ))
        );
        if self.right_pressed && over_view {
            if let Err(error) = menu.studio.focus_camera() {
                std::sync::Arc::make_mut(&mut menu.studio.view).error = Some(error);
            }
            self.looking = true;
            menu.field = None;
        }
        self.looking &= self.right;
        let gesture_action = match hovered {
            Some(MenuAction::Studio(Action::CameraControl {
                shot,
                at,
                handle: recording_studio::camera::PathControl::Key,
                bounds,
            })) if self.held & SHORTCUT != 0 => {
                let handle = if self.held & ALT != 0 {
                    recording_studio::camera::PathControl::Incoming
                } else {
                    recording_studio::camera::PathControl::Outgoing
                };
                Some(MenuAction::Studio(Action::CameraControl {
                    shot,
                    at,
                    handle,
                    bounds,
                }))
            }
            _ => hovered,
        };
        let owned = menu
            .studio
            .camera_pointer(gesture_action, point, held, pressed);
        if !menu.studio.view.camera_workspace.focused {
            self.held &= MODIFIERS;
            self.focused = false;
            self.looking = false;
            return owned;
        }
        if menu.field.is_some() {
            menu.studio.release_camera();
            return owned;
        }
        if !self.focused {
            self.held &= MODIFIERS;
            self.look = [0.0; 2];
        }
        self.focused = true;
        let axis = |positive, negative| {
            f64::from(u8::from(self.held & positive != 0))
                - f64::from(u8::from(self.held & negative != 0))
        };
        let movement = if self.held & SHORTCUT == 0 {
            [axis(RIGHT, LEFT), axis(UP, DOWN), axis(FORWARD, BACKWARD)]
        } else {
            [0.0; 3]
        };
        menu.studio.navigate_camera(
            movement,
            if self.looking { self.look } else { [0.0; 2] },
            elapsed,
            self.held & SHIFT != 0,
            self.looking,
        );
        owned || self.looking
    }
}

/// Only explicit fly keys enter the retained mask; ordinary menu shortcuts keep their own routing.
fn mask(key: KeyCode) -> u16 {
    match key {
        KeyCode::KeyW => FORWARD,
        KeyCode::KeyS => BACKWARD,
        KeyCode::KeyA => LEFT,
        KeyCode::KeyD => RIGHT,
        KeyCode::KeyQ => DOWN,
        KeyCode::KeyE => UP,
        KeyCode::ShiftLeft => SHIFT_LEFT,
        KeyCode::ShiftRight => SHIFT_RIGHT,
        KeyCode::ControlLeft => CTRL_LEFT,
        KeyCode::ControlRight => CTRL_RIGHT,
        KeyCode::SuperLeft => SUPER_LEFT,
        KeyCode::SuperRight => SUPER_RIGHT,
        KeyCode::AltLeft => ALT_LEFT,
        KeyCode::AltRight => ALT_RIGHT,
        _ => 0,
    }
}

/// Navigation releases as soon as the actual painted viewport no longer owns a hit region.
pub(super) fn viewport_visible(presentation: &UiPresentationRuntime) -> bool {
    presentation.menu_focus_actions().any(|action| {
        matches!(
            action,
            launcher::menu::MenuAction::Studio(
                launcher::menu::studio::Action::CameraViewport { .. }
            )
        )
    })
}
