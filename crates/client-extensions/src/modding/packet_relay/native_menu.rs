//! Bounded menu input and retained output on the existing serialized packet worker.

use super::Request;
use mod_api::native_menu::Input;
use mod_host::packet_mod::native_menu::Output;
use std::sync::{
    Arc, Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
};

/// A validated retained panel and its static catalog, prepared outside the frame thread.
pub struct MenuOutput {
    pub output: Output,
    pub prepared: Arc<mod_ui::PreparedPanelUi>,
}

/// The session owns this sender; the app reaches it weakly, without retaining the session.
pub(super) struct MenuLink {
    requests: Sender<Request>,
    pub(super) identity: u64,
    menu: Arc<NativeMenu>,
}

impl MenuLink {
    /// Attaches one mailbox to the packet worker's request channel.
    pub(super) fn new(requests: Sender<Request>, menu: Arc<NativeMenu>, identity: u64) -> Self {
        Self {
            requests,
            menu,
            identity,
        }
    }

    /// Whether the guest negotiated menus and the session still holds its grants.
    pub(super) fn available(&self) -> bool {
        self.menu.available()
    }

    /// Preserves ordered edits and key presses while coalescing the current visibility.
    pub(super) fn exchange(&self, input: Input) -> Option<Arc<MenuOutput>> {
        let (output, wake) = self.menu.exchange(input)?;
        if wake && self.requests.send(Request::NativeMenu).is_err() {
            self.menu.deactivate();
            return None;
        }
        output
    }

    /// Immediately releases pending input and output when the network session ends.
    pub(super) fn deactivate(&self) {
        self.menu.deactivate();
    }
}

#[derive(Default)]
struct State {
    epoch: u64,
    open: bool,
    pending: Option<Input>,
    output: Option<(u64, Arc<MenuOutput>)>,
    wake_pending: bool,
}

/// A pending sample carries the visibility epoch that may accept its result.
pub(super) struct MenuInput {
    pub input: Input,
    epoch: u64,
}

/// A mailbox never owns a sender, so dropping the session also stops its worker.
pub(super) struct NativeMenu {
    state: Mutex<State>,
    ready: AtomicBool,
    stopped: AtomicBool,
    authority: Arc<AtomicBool>,
}

impl NativeMenu {
    /// Starts unavailable until session startup succeeds with an explicit controls grant.
    pub(super) fn new(authority: Arc<AtomicBool>) -> Self {
        Self {
            state: Mutex::new(State::default()),
            ready: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            authority,
        }
    }

    /// Publishes negotiated support without reviving a stopped or replaced session.
    pub(super) fn activate(&self, ready: bool) {
        self.ready.store(
            ready && !self.stopped.load(Ordering::Acquire),
            Ordering::Release,
        );
    }

    /// Stops all menu effects synchronously, including results of an in-flight callback.
    pub(super) fn deactivate(&self) {
        self.ready.store(false, Ordering::Release);
        self.stopped.store(true, Ordering::Release);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.pending = None;
        state.output = None;
        state.wake_pending = false;
        state.epoch = state.epoch.wrapping_add(1);
    }

    /// Current authority is checked at both publication and consumption boundaries.
    fn available(&self) -> bool {
        self.ready.load(Ordering::Acquire)
            && !self.stopped.load(Ordering::Acquire)
            && self.authority.load(Ordering::Acquire)
    }

    /// Returns each state change once and queues at most one worker wake for pending input.
    fn exchange(&self, input: Input) -> Option<(Option<Arc<MenuOutput>>, bool)> {
        if !self.available() || !input.is_valid() {
            return None;
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !self.available() {
            return None;
        }
        if input.open != state.open {
            state.open = input.open;
            state.epoch = state.epoch.wrapping_add(1);
            state.output = None;
            state.pending = None;
        }
        let epoch = state.epoch;
        let output = state
            .output
            .take()
            .filter(|(generation, _)| *generation == epoch)
            .map(|(_, value)| value);
        if let Some(pending) = state.pending.as_mut() {
            if pending.events.len() + input.events.len() > mod_api::MAX_CONTROL_KEYS
                || pending.keys.len() + input.keys.len() > mod_api::MAX_CONTROL_KEYS
            {
                drop(state);
                self.deactivate();
                return None;
            }
            pending.open = input.open;
            pending.viewport = input.viewport;
            pending.events.extend(input.events);
            pending.keys.extend(input.keys);
        } else {
            state.pending = Some(input);
        }
        let wake = !std::mem::replace(&mut state.wake_pending, true);
        Some((output, wake))
    }

    /// The packet worker consumes one bounded input batch between ordinary packet calls.
    pub(super) fn take_input(&self) -> Option<MenuInput> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.wake_pending = false;
        if !self.available() {
            return None;
        }
        state.pending.take().map(|input| MenuInput {
            input,
            epoch: state.epoch,
        })
    }

    /// A close, grant change or stop invalidates an older in-flight presentation update.
    pub(super) fn complete(&self, input: &MenuInput, output: Option<Arc<MenuOutput>>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if self.available() && state.epoch == input.epoch {
            if let Some(output) = output {
                state.output = Some((input.epoch, output));
            }
        }
    }
}

/// Retains a matching static catalog while bindings and control values change.
pub(super) fn prepare(
    output: Output,
    previous: &mut Option<Arc<MenuOutput>>,
) -> Result<Arc<MenuOutput>, String> {
    let prepared = match previous
        .as_ref()
        .filter(|old| old.prepared.route(&output.panel).is_some())
    {
        Some(old) => Arc::clone(&old.prepared),
        None => mod_ui::PreparedPanelUi::prepare(Arc::new(ui::mod_panel::PanelBundle {
            panel: output.panel.clone(),
            surfaces: Default::default(),
        }))?,
    };
    let output = Arc::new(MenuOutput { output, prepared });
    *previous = Some(Arc::clone(&output));
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::native_menu::{Event, Key};

    /// Makes a ready mailbox under revocable source authority.
    fn mailbox() -> (NativeMenu, Arc<AtomicBool>) {
        let authority = Arc::new(AtomicBool::new(true));
        let menu = NativeMenu::new(Arc::clone(&authority));
        menu.activate(true);
        (menu, authority)
    }

    /// Prepares a real native catalog without retaining an executable or asset fixture.
    fn output() -> Arc<MenuOutput> {
        let panel: ui::mod_panel::Panel = serde_json::from_value(serde_json::json!({
            "title":"Menu","toggle_key":"ShiftRight","dark":true,"controls":[],
            "surface":{"screen":"fixture.root","document":serde_json::json!({
                "namespace":"fixture","root":{"type":"panel","controls":[]}
            }).to_string()}
        }))
        .unwrap();
        prepare(
            Output {
                open: true,
                panel,
                keys: vec!["KeyR".into()],
            },
            &mut None,
        )
        .unwrap()
    }

    #[test]
    fn ordered_edits_survive_coalescing_without_queuing_multiple_wakes() {
        let (menu, _) = mailbox();
        let mut first = Input {
            viewport: Some([640.0, 400.0]),
            ..Default::default()
        };
        first.events.push(Event {
            id: "toggle".into(),
            value: 1.0,
        });
        assert!(menu.exchange(first).unwrap().1);
        let mut second = Input {
            viewport: Some([427.0, 240.0]),
            ..Default::default()
        };
        second.keys.push(Key {
            key: "KeyR".into(),
            text: None,
        });
        assert!(!menu.exchange(second).unwrap().1);
        let input = menu.take_input().unwrap();
        assert_eq!(input.input.viewport, Some([427.0, 240.0]));
        assert_eq!(input.input.events[0].id, "toggle");
        assert_eq!(input.input.keys[0].key, "KeyR");
        assert!(menu.take_input().is_none());
    }

    #[test]
    fn visibility_changes_and_revocation_discard_in_flight_presentations() {
        let (menu, authority) = mailbox();
        menu.exchange(Input::default()).unwrap();
        let old = menu.take_input().unwrap();
        menu.exchange(Input {
            open: true,
            ..Default::default()
        })
        .unwrap();
        menu.complete(&old, Some(output()));
        assert!(
            menu.exchange(Input {
                open: true,
                ..Default::default()
            })
            .unwrap()
            .0
            .is_none()
        );
        let current = menu.take_input().unwrap();
        menu.complete(&current, Some(output()));
        authority.store(false, Ordering::Release);
        assert!(
            menu.exchange(Input {
                open: true,
                ..Default::default()
            })
            .is_none()
        );
        menu.deactivate();
        menu.activate(true);
        assert!(!menu.available());
    }

    #[test]
    fn visibility_changes_drop_old_control_actions_before_the_next_callback() {
        let (menu, _) = mailbox();
        menu.exchange(Input {
            events: vec![Event {
                id: "old".into(),
                value: 1.0,
            }],
            ..Default::default()
        })
        .unwrap();
        menu.exchange(Input {
            open: true,
            keys: vec![Key {
                key: "KeyR".into(),
                text: None,
            }],
            ..Default::default()
        })
        .unwrap();
        let input = menu.take_input().unwrap();
        assert!(input.input.events.is_empty());
        assert_eq!(input.input.keys[0].key, "KeyR");
    }

    #[test]
    fn unchanged_static_panels_reuse_the_prepared_catalog() {
        let first = output();
        let mut previous = Some(Arc::clone(&first));
        let mut next = first.output.clone();
        next.open = false;
        let next = prepare(next, &mut previous).unwrap();
        assert!(Arc::ptr_eq(&first.prepared, &next.prepared));
    }
}
