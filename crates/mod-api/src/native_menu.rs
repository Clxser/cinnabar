//! Focused native menu input for trusted packet mods; the guest owns menu behavior.

use serde::{Deserialize, Serialize};

/// Native menu JSON contract version.
pub const VERSION: u32 = 1;
/// The host can display a retained custom panel and deliver its focused input.
pub const CONTROLS: u32 = 2;
/// Largest focused input batch accepted by the native menu callback.
pub const MAX_INPUT_BYTES: usize = 64 * 1024;
/// Largest output, including an escaped bounded panel document.
pub const MAX_OUTPUT_BYTES: usize = 512 * 1024;

/// One native control edit or action, identified by the guest's declared control ID.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub id: String,
    pub value: f32,
}

/// One focused physical key press and its optional composed text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl Key {
    /// Physical key names use the same bounded spelling as the shared controls contract.
    pub fn is_valid(&self) -> bool {
        crate::control_key_valid(&self.key)
            && self
                .text
                .as_ref()
                .is_none_or(|text| text.len() <= 256 && !text.chars().any(char::is_control))
    }
}

/// One input-owning menu sample. Closed menus still poll for guest-requested visibility.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub version: u32,
    pub open: bool,
    /// Available authored GUI units before optional reference-size fitting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[f32; 2]>,
    pub events: Vec<Event>,
    pub keys: Vec<Key>,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            version: VERSION,
            open: false,
            viewport: None,
            events: Vec::new(),
            keys: Vec::new(),
        }
    }
}

impl Input {
    /// Rejects unbounded or malformed input before invoking a packet guest.
    pub fn is_valid(&self) -> bool {
        self.version == VERSION
            && self.viewport.is_none_or(|size| {
                size.iter()
                    .all(|value| value.is_finite() && *value > 0.0 && *value <= 1_000_000.0)
            })
            && self.events.len() <= crate::MAX_CONTROL_KEYS
            && self.keys.len() <= crate::MAX_CONTROL_KEYS
            && self.events.iter().all(|event| {
                !event.id.is_empty()
                    && event.id.len() <= 256
                    && event
                        .id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_-.".contains(&c))
                    && event.value.is_finite()
            })
            && self.keys.iter().all(Key::is_valid)
    }
}
