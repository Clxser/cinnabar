//! Offscreen replay is stepped by its owner and has no window whose closure can end it.

use bevy::window::{ExitCondition, WindowPlugin};

/// Leaves termination to the replay owner and explicit faults instead of absent OS windows.
pub(super) fn plugin() -> WindowPlugin {
    WindowPlugin {
        primary_window: None,
        exit_condition: ExitCondition::DontExit,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::app::{App, AppExit};

    #[test]
    fn offscreen_updates_do_not_request_exit_without_a_window() {
        let mut app = App::new();
        app.add_plugins(plugin());
        for _ in 0..4 {
            app.update();
            assert_eq!(app.should_exit(), None);
        }
    }

    #[test]
    fn explicit_fault_exit_remains_visible_to_the_owner() {
        let mut app = App::new();
        app.add_plugins(plugin());
        app.world_mut().write_message(AppExit::error());
        app.update();
        assert_eq!(app.should_exit(), Some(AppExit::error()));
    }
}
