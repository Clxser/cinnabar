//! Tracks both sides of each editing modifier across consumed keyboard events.

use super::{ButtonInput, ButtonState, KeyCode, KeyboardInput};

#[derive(Default)]
pub struct MenuModifiers(pub(super) u8);

impl MenuModifiers {
    const CONTROL_LEFT: u8 = 1 << 0;
    const CONTROL_RIGHT: u8 = 1 << 1;
    const SUPER_LEFT: u8 = 1 << 2;
    const SUPER_RIGHT: u8 = 1 << 3;
    const ALT_LEFT: u8 = 1 << 4;
    const ALT_RIGHT: u8 = 1 << 5;
    const SHIFT_LEFT: u8 = 1 << 6;
    const SHIFT_RIGHT: u8 = 1 << 7;

    pub(super) fn capture_pressed(&mut self, keys: &ButtonInput<KeyCode>) {
        for key in [
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
            KeyCode::SuperLeft,
            KeyCode::SuperRight,
            KeyCode::AltLeft,
            KeyCode::AltRight,
            KeyCode::ShiftLeft,
            KeyCode::ShiftRight,
        ] {
            if keys.pressed(key) {
                self.0 |= Self::mask(key);
            }
        }
    }

    pub(super) fn observe(&mut self, input: &KeyboardInput) {
        let mask = Self::mask(input.key_code);
        if input.state == ButtonState::Pressed {
            self.0 |= mask;
        } else {
            self.0 &= !mask;
        }
    }

    pub(super) fn shortcut(&self) -> bool {
        self.0 & (Self::CONTROL_LEFT | Self::CONTROL_RIGHT | Self::SUPER_LEFT | Self::SUPER_RIGHT)
            != 0
            && !self.alt()
    }

    /// Alt suppresses editing shortcuts and temporarily bypasses timeline snapping.
    pub(super) fn alt(&self) -> bool {
        self.0 & (Self::ALT_LEFT | Self::ALT_RIGHT) != 0
    }

    pub(super) fn shift(&self) -> bool {
        self.0 & (Self::SHIFT_LEFT | Self::SHIFT_RIGHT) != 0
    }

    const fn mask(key: KeyCode) -> u8 {
        match key {
            KeyCode::ControlLeft => Self::CONTROL_LEFT,
            KeyCode::ControlRight => Self::CONTROL_RIGHT,
            KeyCode::SuperLeft => Self::SUPER_LEFT,
            KeyCode::SuperRight => Self::SUPER_RIGHT,
            KeyCode::AltLeft => Self::ALT_LEFT,
            KeyCode::AltRight => Self::ALT_RIGHT,
            KeyCode::ShiftLeft => Self::SHIFT_LEFT,
            KeyCode::ShiftRight => Self::SHIFT_RIGHT,
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn either_alt_side_bypasses_editing_shortcuts_without_losing_shift() {
        for alt in [KeyCode::AltLeft, KeyCode::AltRight] {
            let mut keys = ButtonInput::default();
            keys.press(KeyCode::ControlLeft);
            keys.press(KeyCode::ShiftRight);
            let mut modifiers = MenuModifiers::default();
            modifiers.capture_pressed(&keys);
            assert!(modifiers.shortcut() && modifiers.shift() && !modifiers.alt());
            keys.press(alt);
            modifiers.capture_pressed(&keys);
            assert!(!modifiers.shortcut() && modifiers.shift() && modifiers.alt());
        }
    }
}
