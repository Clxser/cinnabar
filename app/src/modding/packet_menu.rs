//! Projects the trusted packet guest's retained menu through the shared native panel owner.

use super::packets::PacketMods;
use client_shell::ui_resources::UiPresentationResource;
use mod_api::native_menu::Input;
use mod_ui::PanelUiOwner;
use std::sync::Arc;

/// Input is collected once, then exchanged without blocking the next frame's physical router.
#[derive(Default)]
pub(super) struct State {
    pub owned: bool,
    relay: Option<u64>,
    session: u64,
    pub pending: Input,
    prepared: Option<Arc<mod_ui::PreparedPanelUi>>,
    pub retained: Option<Arc<client_extensions::modding::packet_relay::MenuOutput>>,
}

/// Applies each guest update once; unchanged polling never rebuilds a catalog or panel.
pub(super) fn poll(
    packets: &mut PacketMods,
    presentation: &mut UiPresentationResource,
    session: u64,
    viewport: Option<[f32; 2]>,
) {
    let relay = packets.native_menu_identity();
    if packets.menu.session != session || packets.menu.relay != relay {
        let owner = PanelUiOwner::PacketSession(packets.menu.relay.unwrap_or(0));
        let _ = presentation.mod_ui_mut().set_mod_panel(owner, None);
        presentation.mod_ui_mut().set_prepared_panel_ui(owner, None);
        packets.menu = State {
            session,
            relay,
            ..Default::default()
        };
    }
    if relay.is_none() {
        return;
    }
    let mut input = std::mem::take(&mut packets.menu.pending);
    input.open = packets.menu.owned && presentation.mod_ui().mod_panel_open();
    input.viewport = viewport;
    if let Some(output) = packets.native_menu(input) {
        packets.menu.publish(presentation, output);
    }
    if packets.menu.owned && !presentation.mod_ui().mod_panel_painting() {
        packets.menu.owned = false;
    }
}

/// Reopens the last validated panel locally; the next input informs the guest of visibility.
pub(super) fn toggle(packets: &mut PacketMods, presentation: &mut UiPresentationResource) {
    let Some(retained) = packets.menu.retained.clone() else {
        return;
    };
    let open = !(packets.menu.owned && presentation.mod_ui().mod_panel_open());
    if packets.menu.install(presentation, &retained).is_ok() {
        presentation.mod_ui_mut().set_mod_panel_open(open);
        packets.menu.owned = true;
    }
}

impl State {
    /// Closed background updates retain routes and key subscriptions without taking another UI.
    fn publish(
        &mut self,
        presentation: &mut UiPresentationResource,
        output: Arc<client_extensions::modding::packet_relay::MenuOutput>,
    ) {
        if output.output.open || self.owned {
            if let Err(error) = self.install(presentation, &output) {
                bevy::log::warn!(%error, "Packet menu presentation rejected");
                if self.owned {
                    presentation.mod_ui_mut().set_mod_panel_open(false);
                }
                self.owned = false;
                return;
            }
            presentation
                .mod_ui_mut()
                .set_mod_panel_open(output.output.open);
            self.owned = true;
        }
        self.retained = Some(output);
    }

    /// Publishes a prepared catalog only when this session actually takes panel ownership.
    fn install(
        &mut self,
        presentation: &mut UiPresentationResource,
        output: &Arc<client_extensions::modding::packet_relay::MenuOutput>,
    ) -> Result<(), String> {
        let relay = self.relay.ok_or("packet menu has no live owner")?;
        let owner = PanelUiOwner::PacketSession(relay);
        if !self
            .prepared
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &output.prepared))
        {
            presentation
                .mod_ui_mut()
                .set_prepared_panel_ui(owner, Some(Arc::clone(&output.prepared)));
            self.prepared = Some(Arc::clone(&output.prepared));
        }
        presentation
            .mod_ui_mut()
            .set_mod_panel(owner, Some(&output.output.panel))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prepares a real retained panel without starting an executable or network session.
    fn output(open: bool) -> Arc<client_extensions::modding::packet_relay::MenuOutput> {
        let mut panel = mod_ui::test_support::sample_panel();
        panel.toggle_key = "F8".into();
        let prepared = mod_ui::PreparedPanelUi::prepare(Arc::new(ui::mod_panel::PanelBundle {
            panel: panel.clone(),
            surfaces: Default::default(),
        }))
        .unwrap();
        Arc::new(client_extensions::modding::packet_relay::MenuOutput {
            output: mod_host::packet_mod::native_menu::Output {
                open,
                panel,
                keys: Vec::new(),
            },
            prepared,
        })
    }

    #[test]
    fn closed_background_updates_preserve_another_owners_open_panel() {
        let mut presentation =
            UiPresentationResource(client_ui::test_support::mini_engine_presentation());
        let mut regular = mod_ui::test_support::sample_panel();
        regular.toggle_key = "F10".into();
        presentation
            .mod_ui_mut()
            .set_mod_panel(PanelUiOwner::Environment(0), Some(&regular))
            .unwrap();
        presentation.mod_ui_mut().set_mod_panel_open(true);
        let mut state = State {
            relay: Some(1),
            ..Default::default()
        };
        state.publish(&mut presentation, output(false));
        assert!(presentation.mod_ui().mod_panel_open());
        assert_eq!(presentation.mod_ui().mod_panel_toggle_key(), Some("F10"));
        assert!(!state.owned);
        state.publish(&mut presentation, output(true));
        assert!(state.owned && presentation.mod_ui().mod_panel_open());
        assert_eq!(presentation.mod_ui().mod_panel_toggle_key(), Some("F8"));
    }

    #[test]
    fn stopped_closed_sessions_release_their_registered_catalogs() {
        let mut presentation =
            UiPresentationResource(client_ui::test_support::mini_engine_presentation());
        let output = output(false);
        let retired = Arc::downgrade(&output.prepared);
        presentation.mod_ui_mut().set_prepared_panel_ui(
            PanelUiOwner::PacketSession(7),
            Some(Arc::clone(&output.prepared)),
        );
        let mut packets = PacketMods::default();
        packets.menu = State {
            session: 10,
            relay: Some(7),
            prepared: Some(Arc::clone(&output.prepared)),
            retained: Some(output),
            ..Default::default()
        };
        poll(&mut packets, &mut presentation, 10, None);
        assert!(retired.upgrade().is_none());
        assert!(packets.menu.retained.is_none());
    }
}
