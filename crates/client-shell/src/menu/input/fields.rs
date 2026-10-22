//! Menu text editors share their construction and byte limits.
use crate::menu::{MAX_SERVER_ADDRESS_BYTES, MAX_SERVER_NAME_BYTES, MAX_SERVER_PORT_BYTES};
use launcher::menu::view::MenuCaret;
use launcher::{
    local_worlds::form::{MAX_SEED_CHARS, MAX_WORLD_NAME_CHARS},
    menu::MenuField,
};
use ui::ChatEditor;

/// Creates an empty text editor with the byte limit for its menu field.
pub(in crate::menu) fn field_editor(field: MenuField) -> ChatEditor {
    if matches!(
        field,
        MenuField::StudioTitle
            | MenuField::StudioDestination
            | MenuField::StudioCameraViewName(_)
            | MenuField::StudioSubjectSearch
    ) {
        return ChatEditor::with_byte_budget(
            std::num::NonZeroUsize::new(max_bytes(field)).expect("positive title budget"),
        );
    }
    ChatEditor::new(max_bytes(field)).expect("menu field budgets are valid editor bounds")
}

/// A field's byte budget; world fields allow their vanilla character limits in any script.
fn max_bytes(field: MenuField) -> usize {
    match field {
        MenuField::FriendQuery => ui::MAX_CHAT_INPUT_BYTES,
        MenuField::RealmCode => MAX_SERVER_ADDRESS_BYTES,
        MenuField::Name => MAX_SERVER_NAME_BYTES,
        MenuField::Address => MAX_SERVER_ADDRESS_BYTES,
        MenuField::Port => MAX_SERVER_PORT_BYTES,
        MenuField::WorldName => MAX_WORLD_NAME_CHARS * 4,
        MenuField::WorldSeed => MAX_SEED_CHARS,
        MenuField::SkinName => launcher::dressing_room::MAX_SKIN_NAME_BYTES,
        MenuField::MacroCommand(_) => ui::MAX_CHAT_INPUT_BYTES,
        MenuField::StudioTitle => recording_studio::title::MAX_TITLE_TEXT_BYTES,
        MenuField::StudioDestination => recording_studio::settings::MAX_PATH_BYTES,
        MenuField::StudioCamera(_) => recording_studio::camera::MAX_CAMERA_VALUE_BYTES,
        MenuField::StudioGraph(_) => recording_studio::camera::MAX_CAMERA_VALUE_BYTES,
        MenuField::StudioCameraViewName(_) => recording_studio::camera::MAX_SAVED_VIEW_NAME_BYTES,
        MenuField::StudioSubjectSearch => recording_studio::camera::MAX_TARGET_NAME_BYTES,
        MenuField::StudioClip(_) => launcher::menu::studio::MAX_CLIP_TIME_BYTES,
        MenuField::StudioExportRange(_) => launcher::menu::studio::MAX_CLIP_TIME_BYTES,
        MenuField::StudioSeek => recording_studio::playback::monitoring::MAX_SEEK_BYTES,
        MenuField::StudioLibrarySearch => recording_studio::storage::MAX_LIBRARY_SEARCH_BYTES,
    }
}

impl crate::menu::MenuRuntime {
    /// Borrows the single retained text owner for a menu field.
    pub(super) fn editor(&self, field: MenuField) -> &ChatEditor {
        match field {
            MenuField::FriendQuery => &self.friend_query,
            MenuField::RealmCode => &self.realm_membership.code,
            MenuField::Name => &self.name,
            MenuField::Address => &self.address,
            MenuField::Port => &self.port,
            MenuField::WorldName => &self.local_ui.name,
            MenuField::WorldSeed => &self.local_ui.seed,
            MenuField::SkinName => &self.skin_name,
            MenuField::MacroCommand(slot) => &self.macro_commands[slot],
            MenuField::StudioTitle => &self.studio.title_text,
            MenuField::StudioDestination => &self.studio.destination_text,
            MenuField::StudioCamera(_) => &self.studio.camera_text,
            MenuField::StudioGraph(_) => &self.studio.graph_text,
            MenuField::StudioCameraViewName(_) => &self.studio.camera_view_text,
            MenuField::StudioSubjectSearch => &self.studio.subject_text,
            MenuField::StudioClip(_) => &self.studio.clip_text,
            MenuField::StudioExportRange(_) => &self.studio.range_text,
            MenuField::StudioSeek => &self.studio.seek_text,
            MenuField::StudioLibrarySearch => &self.studio.search_text,
        }
    }

    /// Editing and caret operations share the same retained field owner.
    pub(super) fn editor_mut(&mut self, field: MenuField) -> &mut ChatEditor {
        match field {
            MenuField::FriendQuery => &mut self.friend_query,
            MenuField::RealmCode => &mut self.realm_membership.code,
            MenuField::Name => &mut self.name,
            MenuField::Address => &mut self.address,
            MenuField::Port => &mut self.port,
            MenuField::WorldName => &mut self.local_ui.name,
            MenuField::WorldSeed => &mut self.local_ui.seed,
            MenuField::SkinName => &mut self.skin_name,
            MenuField::MacroCommand(slot) => &mut self.macro_commands[slot],
            MenuField::StudioTitle => &mut self.studio.title_text,
            MenuField::StudioDestination => &mut self.studio.destination_text,
            MenuField::StudioCamera(_) => &mut self.studio.camera_text,
            MenuField::StudioGraph(_) => &mut self.studio.graph_text,
            MenuField::StudioCameraViewName(_) => &mut self.studio.camera_view_text,
            MenuField::StudioSubjectSearch => &mut self.studio.subject_text,
            MenuField::StudioClip(_) => &mut self.studio.clip_text,
            MenuField::StudioExportRange(_) => &mut self.studio.range_text,
            MenuField::StudioSeek => &mut self.studio.seek_text,
            MenuField::StudioLibrarySearch => &mut self.studio.search_text,
        }
    }

    /// `field`'s current text.
    pub fn field_text(&self, field: MenuField) -> &str {
        self.editor(field).as_str()
    }

    pub(in crate::menu) fn caret(&self) -> MenuCaret {
        MenuCaret {
            byte: self
                .field
                .map_or(0, |field| self.editor(field).cursor_byte()),
            selection: self
                .field
                .and_then(|field| self.editor(field).selection())
                .map(|range| [range.start, range.end]),
            revision: self.caret_revision,
            shown: true,
        }
    }
}

impl crate::menu::MenuRuntime {
    /// Focus `field` with its caret after the text, as a newly selected box.
    pub(in crate::menu) fn focus_field(&mut self, field: MenuField) {
        if !matches!(field, MenuField::StudioGraph(_)) {
            self.studio.cancel_graph_value();
        }
        if let MenuField::StudioGraph(parameter) = field
            && let Err(error) = self.studio.begin_graph_value(parameter)
        {
            std::sync::Arc::make_mut(&mut self.studio.view).error = Some(error);
            self.field = None;
            return;
        }
        if field == MenuField::StudioSubjectSearch {
            self.studio.begin_camera_subject_search();
        }
        if !matches!(field, MenuField::StudioCameraViewName(_)) {
            self.studio.cancel_camera_view_name();
        }
        if let MenuField::StudioCameraViewName(id) = field
            && let Err(error) = self.studio.begin_camera_view_name(id)
        {
            std::sync::Arc::make_mut(&mut self.studio.view).error = Some(error);
            self.field = None;
            return;
        }
        if field != MenuField::StudioLibrarySearch {
            self.studio.cancel_library_search();
        }
        if field == MenuField::StudioLibrarySearch {
            self.studio.begin_library_search();
        }
        if !matches!(field, MenuField::StudioExportRange(_)) {
            self.studio.cancel_export_range();
        }
        if let MenuField::StudioExportRange(parameter) = field
            && let Err(error) = self.studio.begin_export_range(parameter)
        {
            std::sync::Arc::make_mut(&mut self.studio.view).error = Some(error);
            self.field = None;
            return;
        }
        if field != MenuField::StudioSeek {
            self.studio.cancel_monitor_seek();
        }
        if field == MenuField::StudioSeek
            && let Err(error) = self.studio.begin_monitor_seek()
        {
            std::sync::Arc::make_mut(&mut self.studio.view).error = Some(error);
            self.field = None;
            return;
        }
        if !matches!(field, MenuField::StudioClip(_)) {
            self.studio.cancel_clip_time();
        }
        if let MenuField::StudioClip(parameter) = field
            && let Err(error) = self.studio.begin_clip_time(parameter)
        {
            std::sync::Arc::make_mut(&mut self.studio.view).error = Some(error);
            self.field = None;
            return;
        }
        if field == MenuField::StudioDestination {
            self.studio.begin_destination();
        }
        if let MenuField::StudioCamera(parameter) = field
            && let Err(error) = self.studio.begin_camera_value(parameter)
        {
            std::sync::Arc::make_mut(&mut self.studio.view).error = Some(error);
            self.field = None;
            return;
        }
        if field == MenuField::StudioTitle
            && let Err(error) = self.studio.begin_title_text()
        {
            std::sync::Arc::make_mut(&mut self.studio.view).error = Some(error);
            self.field = None;
            return;
        }
        self.field = Some(field);
        self.edit_field(|editor| editor.move_end(false));
    }

    /// Native text input owns key priority while any field holds the caret.
    pub(in crate::menu) fn has_focused_field(&self) -> bool {
        self.field.is_some()
    }

    /// Apply `edit` to the focused field; every edit or caret move restarts the blink,
    /// as vanilla's text edit box shows its caret again after typing.
    pub(in crate::menu) fn edit_field(&mut self, edit: impl FnOnce(&mut ChatEditor)) {
        let Some(field) = self.field else {
            return;
        };
        if field == MenuField::SkinName && self.dressing_room.busy {
            return;
        }
        edit(self.editor_mut(field));
        if matches!(field, MenuField::StudioGraph(_)) {
            self.studio.publish_graph_value();
        }
        if field == MenuField::StudioLibrarySearch {
            self.studio.publish_library_search();
        }
        if field == MenuField::StudioSeek {
            self.studio.publish_monitor_seek();
        }
        if field == MenuField::StudioDestination {
            self.studio.publish_destination();
        }
        if field == MenuField::StudioTitle {
            self.studio.publish_title_text();
        }
        if matches!(field, MenuField::StudioCamera(_)) {
            self.studio.publish_camera_value();
        }
        if matches!(field, MenuField::StudioCameraViewName(_)) {
            self.studio.publish_camera_view_name();
        }
        if field == MenuField::StudioSubjectSearch {
            self.studio.publish_camera_subject_search();
        }
        if matches!(field, MenuField::StudioClip(_)) {
            self.studio.publish_clip_time();
        }
        if matches!(field, MenuField::StudioExportRange(_)) {
            self.studio.publish_export_range();
        }
        if let MenuField::MacroCommand(slot) = field {
            self.sync_macro_command(slot);
        }
        if field == MenuField::RealmCode
            && let Some(state) = &mut self.realm_membership.state
        {
            state.code = self.realm_membership.code.as_str().to_owned();
            state.error = None;
        }
        self.caret_revision = self.caret_revision.wrapping_add(1);
        if field == MenuField::SkinName {
            self.sync_skin_name_draft();
        }
    }
}
