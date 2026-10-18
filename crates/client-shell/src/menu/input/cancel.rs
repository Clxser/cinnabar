//! Routes cancel input through selected controls before leaving the menu.

use super::*;

impl MenuRuntime {
    /// A selected vanilla edit box consumes cancel before the screen handles it.
    pub(super) fn go_back_from_input(&mut self) {
        if self.screen == launcher::menu::MenuScreen::RecordingStudio {
            if self.field == Some(MenuField::StudioLibrarySearch) {
                self.studio.cancel_library_search();
                self.field = None;
                return;
            }
            if self.studio.view.workflow.recovery.visible {
                self.go_back();
                return;
            }
            if self.studio.view.camera_workspace.focused {
                self.studio.release_camera();
                return;
            }
            if self.studio.view.discard.is_some() {
                self.studio
                    .activate(launcher::menu::studio::Action::CancelDiscard);
                return;
            }
            if self.studio.view.loading {
                self.studio
                    .activate(launcher::menu::studio::Action::CancelOpen);
                return;
            }
        }
        if self.sign_in_focus().is_some() || self.join_request_prompted() {
            self.go_back();
            return;
        }
        if self.clear_settings_slider_selection() {
            return;
        }
        if self.screen == launcher::menu::MenuScreen::Inbox
            && (self.feeds.inbox_state.opened.is_some()
                || self.feeds.inbox_state.delete_pending.is_some()
                || self.feeds.inbox_state.filters)
        {
            self.activate_inbox(launcher::menu::inbox::Action::Cancel);
            return;
        }
        if self.screen == launcher::menu::MenuScreen::AddServer && self.field.is_some() {
            self.edit_field(|editor| editor.place_cursor(editor.cursor_byte()));
            self.field = None;
            return;
        }
        self.go_back();
    }
}
