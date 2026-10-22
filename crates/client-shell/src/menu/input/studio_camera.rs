//! Physical key state is retained independently of the menu's per-frame ButtonInput resets.

use super::*;
use bevy::input::mouse::MouseMotion;
use launcher::menu::{
    MenuAction, MenuScreen,
    studio::{Action, NavigationGesture},
};
use recording_studio::camera::navigation::LOOK_DEGREES_PER_PIXEL;

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
    middle: bool,
    middle_pressed: bool,
    panning: bool,
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
            self.middle = false;
            self.panning = false;
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
        self.middle_pressed = false;
        if let Some(mouse) = mouse {
            for input in self.mouse.read(mouse) {
                if input.button == MouseButton::Right {
                    self.right = available && input.state == ButtonState::Pressed;
                    self.right_pressed |= self.right;
                }
                if input.button == MouseButton::Middle {
                    self.middle = available && input.state == ButtonState::Pressed;
                    self.middle_pressed |= self.middle;
                }
            }
        }
        self.look = [0.0; 2];
        if let Some(motion) = motion {
            for input in self.motion.read(motion) {
                self.look[0] += f64::from(input.delta.x) * LOOK_DEGREES_PER_PIXEL;
                self.look[1] += f64::from(input.delta.y) * LOOK_DEGREES_PER_PIXEL;
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
        viewport: Option<[i32; 4]>,
    ) -> bool {
        if viewport.is_none() || !available(menu) {
            menu.studio.release_camera();
            self.panning = false;
            return false;
        }
        let hovered = menu.hovered;
        let over_view = image_hit(hovered);
        if !menu.studio.authored_camera_drag_active()
            && (self.panning || (self.middle_pressed && over_view && menu.field.is_none()))
        {
            let pressed = self.middle_pressed && !self.panning;
            self.panning = menu
                .studio
                .pan_camera(point, self.middle, pressed, viewport)
                && self.middle;
            self.looking = false;
            self.look = [0.0; 2];
            return true;
        }
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

    /// Hover ownership is checked separately from keyboard focus so timeline scrolling remains natural.
    pub(super) fn wheel(
        &self,
        menu: &mut MenuRuntime,
        vertical: f32,
        pixels: bool,
        zoom: bool,
    ) -> bool {
        if !available(menu) || menu.field.is_some() || !image_hit(menu.hovered) {
            return false;
        }
        menu.studio
            .camera_wheel(vertical, pixels, NavigationGesture::wheel(zoom, self.right))
    }
}

/// Overlays are camera-image hits too; ordinary controls and timeline clips retain their own input.
fn image_hit(action: Option<MenuAction>) -> bool {
    matches!(
        action,
        Some(MenuAction::Studio(
            Action::CameraViewport { .. }
                | Action::CameraControl { .. }
                | Action::CameraAxisControl { .. }
                | Action::CameraSubject(_)
        ))
    )
}

/// All physical camera gestures obey the same modal and route admission.
fn available(menu: &MenuRuntime) -> bool {
    menu.screen() == MenuScreen::RecordingStudio
        && menu.dialog.is_none()
        && menu.studio.camera_navigation_available()
        && !menu.join_request_prompted()
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

/// The painted fitted image supplies projection geometry; letterbox space never becomes a hit.
pub(super) fn viewport_bounds(presentation: &UiPresentationRuntime) -> Option<[i32; 4]> {
    presentation
        .menu_focus_actions()
        .find_map(|action| match action {
            MenuAction::Studio(Action::CameraViewport { bounds }) => Some(bounds),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use recording_studio::{
        Id, MediaTime,
        camera::{CameraDragAxis, PathControl},
    };

    #[test]
    fn camera_overlay_hits_include_axis_handles_and_exclude_the_timeline() {
        let axis = MenuAction::Studio(Action::CameraAxisControl {
            shot: Id(1),
            at: MediaTime::ZERO,
            handle: PathControl::Key,
            axis: CameraDragAxis::X,
            bounds: [0, 0, 100, 100],
        });
        assert!(image_hit(Some(axis)));
        assert!(image_hit(Some(MenuAction::Studio(
            Action::CameraViewport {
                bounds: [0, 0, 100, 100]
            }
        ))));
        for action in [
            Action::Scrub {
                left: 0,
                width: 100,
            },
            Action::Step(1),
            Action::Panel(launcher::menu::studio::EditorPanel::Media),
        ] {
            assert!(!image_hit(Some(MenuAction::Studio(action))));
        }
        assert!(!image_hit(None));
    }

    #[test]
    fn unavailable_window_clears_middle_capture_and_retained_fly_keys() {
        let mut input = Input {
            held: FORWARD | SHIFT_LEFT,
            middle: true,
            panning: true,
            right: true,
            looking: true,
            focused: true,
            ..Default::default()
        };
        input.observe(None, None, None, false, false);
        assert_eq!(input.held, 0);
        assert!(!input.middle && !input.panning && !input.right && !input.looking);
    }

    #[test]
    fn wheel_does_not_capture_timeline_text_fields_or_hidden_workspaces() {
        let input = Input::default();
        let mut menu = MenuRuntime::new(true, 2, "Fixture".into());
        menu.screen = MenuScreen::RecordingStudio;
        std::sync::Arc::make_mut(&mut menu.studio.view).show_library = false;
        menu.hovered = Some(MenuAction::Studio(Action::Scrub {
            left: 0,
            width: 100,
        }));
        assert!(!input.wheel(&mut menu, 1.0, false, false));
        menu.hovered = Some(MenuAction::Studio(Action::CameraViewport {
            bounds: [0, 0, 100, 100],
        }));
        menu.field = Some(launcher::menu::MenuField::StudioSeek);
        assert!(!input.wheel(&mut menu, 1.0, false, false));
        menu.field = None;
        std::sync::Arc::make_mut(&mut menu.studio.view)
            .workflow
            .show_settings = true;
        assert!(!input.wheel(&mut menu, 1.0, false, false));
    }

    #[test]
    fn visible_or_busy_recovery_rejects_pointer_and_wheel_navigation() {
        for (visible, busy) in [(true, false), (false, true)] {
            let mut menu = MenuRuntime::new(true, 2, "Fixture".into());
            menu.screen = MenuScreen::RecordingStudio;
            let view = std::sync::Arc::make_mut(&mut menu.studio.view);
            view.show_library = false;
            view.workflow.recovery.visible = visible;
            view.workflow.recovery.busy = busy;
            menu.hovered = Some(MenuAction::Studio(Action::CameraViewport {
                bounds: [0, 0, 100, 100],
            }));
            let mut input = Input {
                middle: true,
                middle_pressed: true,
                ..Default::default()
            };
            assert!(!input.wheel(&mut menu, 1.0, false, false));
            assert!(!input.drive(
                &mut menu,
                Some([50.0; 2]),
                false,
                false,
                0.05,
                Some([0, 0, 100, 100])
            ));
            assert!(!menu.studio.view.camera_workspace.focused);
        }
    }
}
