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
    if matches!(field, MenuField::StudioTitle | MenuField::StudioDestination) {
        return ChatEditor::with_byte_budget(
            std::num::NonZeroUsize::new(max_bytes(field)).expect("positive title budget"),
        );
    }
    ChatEditor::new(max_bytes(field)).expect("menu field budgets are valid editor bounds")
}

/// A field's byte budget; world fields allow their vanilla character limits in any script.
fn max_bytes(field: MenuField) -> usize {
    match field {
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
        MenuField::StudioClip(_) => launcher::menu::studio::MAX_CLIP_TIME_BYTES,
    }
}

impl crate::menu::MenuRuntime {
    /// Borrows the single retained text owner for a menu field.
    pub(super) fn editor(&self, field: MenuField) -> &ChatEditor {
        match field {
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
            MenuField::StudioClip(_) => &self.studio.clip_text,
        }
    }

    /// Editing and caret operations share the same retained field owner.
    pub(super) fn editor_mut(&mut self, field: MenuField) -> &mut ChatEditor {
        match field {
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
            MenuField::StudioClip(_) => &mut self.studio.clip_text,
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
