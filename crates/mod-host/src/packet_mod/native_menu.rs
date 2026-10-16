//! Retained panel output validation for the optional native packet-menu callback.

use super::PacketMod;
use anyhow::{Result, ensure};
use mod_api::native_menu::{Input, MAX_INPUT_BYTES, MAX_OUTPUT_BYTES};
use serde::{Deserialize, Serialize};

/// A complete retained panel update; JSON null leaves the existing presentation unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub open: bool,
    pub panel: ui::mod_panel::Panel,
    /// Physical keys reserved for guest-owned module actions while gameplay owns input.
    #[serde(default)]
    pub keys: Vec<String>,
}

impl PacketMod {
    /// Whether a live guest negotiated the explicitly granted native controls callback.
    pub fn has_native_menu(&self) -> bool {
        self.is_active() && self.guest.has_native_menu()
    }

    /// Calls the guest once and validates every returned panel before publication.
    pub fn menu(&mut self, input: &Input) -> Result<Option<Output>> {
        if !self.has_native_menu() {
            return Ok(None);
        }
        ensure!(
            input.is_valid()
                && input
                    .events
                    .iter()
                    .all(|event| event.id.len() <= ui::mod_panel::MAX_PANEL_ID_BYTES),
            "invalid native menu input"
        );
        let json = serde_json::to_vec(input)?;
        ensure!(
            json.len() <= MAX_INPUT_BYTES,
            "native menu input exceeds its byte limit"
        );
        let deadline = self.call_deadline;
        self.call("menu", deadline, |guest| {
            let bytes = match guest.menu(&json)? {
                Ok(Some(bytes)) => bytes,
                Ok(None) => return Ok(Ok(None)),
                Err(error) => return Ok(Err(error)),
            };
            ensure!(
                bytes.len() <= MAX_OUTPUT_BYTES,
                "native menu output exceeds its byte limit"
            );
            let output: Option<Output> = serde_json::from_slice(&bytes)?;
            if let Some(output) = &output {
                output.panel.validate().map_err(anyhow::Error::msg)?;
                ensure!(
                    mod_api::control_key_valid(&output.panel.toggle_key)
                        && output.keys.len() <= mod_api::MAX_CONTROL_KEYS
                        && output
                            .keys
                            .iter()
                            .all(|key| mod_api::control_key_valid(key)),
                    "invalid native menu key subscriptions"
                );
            }
            Ok(Ok(output))
        })
        .map(Option::flatten)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModGrants, packet_mod::CompiledPacketMod};

    /// Appends the optional menu ABI to the ordinary packet-relay fixture.
    fn module(output: &str, length: usize) -> Vec<u8> {
        let mut source = include_str!("tests/guest.wat")
            .replace("$IMPORTS", "")
            .replace("$START", "");
        source = source.trim_end().strip_suffix(')').unwrap().to_owned();
        let output: String = output.bytes().map(|byte| format!("\\{byte:02x}")).collect();
        source.push_str(&format!(
            r#"
            (data (i32.const 2048) "{output}")
            (func (export "native-init") (param i32)
                (i32.store (i32.const 2000) (local.get 0)))
            (func (export "menu") (param i32 i32) (result i32)
                (if (i32.ne (i32.load (i32.const 2000)) (i32.const 2)) (then unreachable))
                (i32.store (i32.const 128) (i32.const 0))
                (i32.store (i32.const 132) (i32.const 2048))
                (i32.store (i32.const 136) (i32.const {length}))
                (i32.const 128)))
        "#
        ));
        wat::parse_str(source).unwrap()
    }

    /// Only the controls grant activates the menu export, independently of camera support.
    fn instance(output: &str, length: usize, controls: bool) -> PacketMod {
        CompiledPacketMod::compile(
            &module(output, length),
            &ModGrants {
                packets: true,
                controls,
                ..Default::default()
            },
            None,
        )
        .unwrap()
        .instantiate()
        .unwrap()
    }

    /// A minimal authored packet menu using the production panel schema.
    fn output() -> String {
        serde_json::json!({"open":true,"keys":["KeyR"],"panel":{
            "title":"Menu","toggle_key":"ShiftRight","dark":true,
            "controls":[{"kind":"button","id":"action","label":"Action"}],
            "surface":{"screen":"fixture.root","document":serde_json::json!({
                "namespace":"fixture","root":{"type":"panel","controls":[{"action":{
                    "type":"button","size":[60,20],"button_mappings":[{
                        "from_button_id":"button.menu_select","mapping_type":"pressed",
                        "to_button_id":"mod.control:0"}]
                }}]}}).to_string()}
        }})
        .to_string()
    }

    #[test]
    fn menu_only_reactors_negotiate_controls_and_retain_valid_panels() {
        let output = output();
        let mut guest = instance(&output, output.len(), true);
        assert!(guest.has_native_menu());
        assert!(!guest.has_native_frame());
        let menu = guest.menu(&Input::default()).unwrap().unwrap();
        assert!(menu.open);
        assert_eq!(menu.keys, ["KeyR"]);
        assert_eq!(menu.panel.controls[0].id(), "action");
        let mut denied = instance(&output, output.len(), false);
        assert!(!denied.has_native_menu());
        assert_eq!(denied.menu(&Input::default()).unwrap(), None);
        assert!(denied.is_active());
    }

    #[test]
    fn unchanged_menus_return_null_without_clearing_the_guest() {
        let mut guest = instance("null", 4, true);
        assert_eq!(guest.menu(&Input::default()).unwrap(), None);
        assert!(guest.is_active());
    }

    #[test]
    fn invalid_panel_actions_and_output_sizes_quarantine_the_guest() {
        let invalid = output().replace("mod.control:0", "mod.control:1");
        let mut guest = instance(&invalid, invalid.len(), true);
        assert!(guest.menu(&Input::default()).is_err());
        assert!(!guest.is_active());
        let mut guest = instance("null", MAX_OUTPUT_BYTES + 1, true);
        assert!(guest.menu(&Input::default()).is_err());
        assert!(!guest.is_active());
        let invalid = output().replace("KeyR", "not a physical key");
        let mut guest = instance(&invalid, invalid.len(), true);
        assert!(guest.menu(&Input::default()).is_err());
        assert!(!guest.is_active());
    }
    #[test]
    fn real_go_reactor_opens_edits_and_closes_a_native_menu_with_only_controls_granted() {
        let (Some(wasm), Some(capture)) = (
            std::env::var_os("LUNAR_NATIVE_WASM"),
            std::env::var_os("LUNAR_NATIVE_CAPTURE"),
        ) else {
            eprintln!(
                "missing fixture: LUNAR_NATIVE_WASM and LUNAR_NATIVE_CAPTURE; skipping real Go native menu integration"
            );
            return;
        };
        let records = crate::packet_mod::capture::parse(
            &std::fs::read(capture).expect("read synthetic packet capture"),
        )
        .unwrap();
        let (identity, client_data, startup) = records
            .iter()
            .find_map(|record| match record {
                crate::packet_mod::capture::Record::Start {
                    identity,
                    client_data,
                    startup,
                } => Some((identity, client_data, startup)),
                _ => None,
            })
            .expect("capture needs a start record");
        let mut guest = CompiledPacketMod::load(
            std::path::Path::new(&wasm),
            &ModGrants {
                packets: true,
                wasi: true,
                controls: true,
                ..Default::default()
            },
            None,
        )
        .unwrap()
        .instantiate()
        .unwrap();
        assert!(guest.has_native_menu());
        assert!(
            !guest.has_native_frame(),
            "camera support is independently denied"
        );
        guest.start(identity, client_data, startup).unwrap();
        assert!(guest.menu(&Input::default()).unwrap().is_none());
        let mut input = Input {
            open: true,
            ..Default::default()
        };
        let menu = guest
            .menu(&input)
            .unwrap()
            .expect("opening produces a native panel");
        assert!(menu.open);
        let modules = menu
            .panel
            .controls
            .iter()
            .find(|control| control.label() == "Modules")
            .unwrap()
            .id()
            .to_owned();
        input.events.push(mod_api::native_menu::Event {
            id: modules,
            value: 1.0,
        });
        let modules = guest.menu(&input).unwrap().unwrap();
        assert!(modules.open);
        assert!(
            modules
                .panel
                .surface
                .as_ref()
                .unwrap()
                .document
                .contains("cinnabar_glass")
        );
        input.events.clear();
        input.keys.push(mod_api::native_menu::Key {
            key: "Escape".into(),
            text: None,
        });
        let _ = guest.menu(&input).unwrap();
        let closed = guest
            .menu(&input)
            .unwrap()
            .expect("second Escape closes the launcher");
        assert!(!closed.open);
        if let Some(command) = std::env::var_os("LUNAR_NATIVE_OPEN_COMMAND") {
            let command = std::fs::read(command).expect("read encoded native menu command fixture");
            guest
                .on_batch(crate::packet_mod::Direction::FromClient, &command)
                .unwrap();
            let reopened = guest
                .menu(&Input::default())
                .unwrap()
                .expect("/.ui opens the native override");
            assert!(reopened.open);
        }
        assert!(guest.is_active());
        guest.stop();
        assert!(!guest.has_native_menu());
    }
}
