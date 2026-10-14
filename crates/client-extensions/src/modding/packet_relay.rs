//! The selected trusted packet mod: compiled off the frame thread, then run for each session on a
//! thread of its own that serves the session's packet relay.

use mod_host::{
    ModGrants,
    packet_mod::{CompiledPacketMod, Direction, PacketMod, SessionIdentity},
};
use protocol::{PacketRelay, RelayOutput, RelayPack};
use std::{
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex, PoisonError,
        mpsc::{self, Receiver, Sender, SyncSender},
    },
    thread,
    time::Duration,
};

/// How long a starting session waits for the selected packet mod to finish compiling before
/// it continues without it.
pub const PACKET_MOD_READY_WAIT: Duration = Duration::from_secs(20);

/// Where the packet mod comes from and the authority it runs with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PacketModSelection {
    pub path: PathBuf,
    pub grants: ModGrants,
}

/// Compiles the selected packet mod in the background and hands each session a relay.
#[derive(Clone, Default)]
pub struct PacketModLoader {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    cache: Option<PathBuf>,
    state: Mutex<LoaderState>,
    ready: Condvar,
}

#[derive(Default)]
struct LoaderState {
    selection: Option<PacketModSelection>,
    compiled: Option<Result<Arc<CompiledPacketMod>, String>>,
    generation: u64,
}

impl PacketModLoader {
    /// A loader that keeps compiled code in `cache`, a directory this user owns.
    pub fn new(cache: Option<PathBuf>) -> Self {
        Self {
            inner: Arc::new(Inner {
                cache,
                ..Inner::default()
            }),
        }
    }

    /// Selects the packet mod later sessions use and compiles it on a background thread; `None`
    /// clears it. Selecting the current source again keeps its compiled code.
    pub fn select(&self, selection: Option<PacketModSelection>) {
        let mut state = self.state();
        if state.selection == selection {
            return;
        }
        state.generation += 1;
        state.selection.clone_from(&selection);
        state.compiled = None;
        let Some(selection) = selection else {
            return;
        };
        let generation = state.generation;
        drop(state);
        let loader = self.clone();
        let compile = move || {
            let cache = loader.inner.cache.as_deref();
            let result = CompiledPacketMod::load(&selection.path, &selection.grants, cache)
                .map(Arc::new)
                .map_err(|error| format!("{error:#}"));
            if let Err(error) = &result {
                tracing::warn!(
                    path = %selection.path.display(),
                    error,
                    "packet mod disabled"
                );
            }
            let mut state = loader.state();
            if state.generation == generation {
                state.compiled = Some(result);
                loader.inner.ready.notify_all();
            }
        };
        if let Err(error) = thread::Builder::new()
            .name("packet-mod-compile".into())
            .spawn(compile)
        {
            tracing::warn!(%error, "packet mod compile thread unavailable");
        }
    }

    /// A relay for one new session, or `None` when no packet mod is selected.
    pub fn session_relay(&self) -> Option<Arc<dyn PacketRelay>> {
        self.state().selection.as_ref()?;
        Some(Arc::new(PacketModRelay::spawn(self.clone())))
    }

    /// Waits up to `wait` for the selected packet mod's compiled code.
    fn compiled(&self, wait: Duration) -> Option<Arc<CompiledPacketMod>> {
        let state = self.state();
        let (state, _) = self
            .inner
            .ready
            .wait_timeout_while(state, wait, |state| {
                state.selection.is_some() && state.compiled.is_none()
            })
            .unwrap_or_else(PoisonError::into_inner);
        match &state.compiled {
            Some(Ok(compiled)) => Some(Arc::clone(compiled)),
            _ => None,
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, LoaderState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// One call the session makes, answered by the packet mod's thread.
enum Request {
    Start {
        identity: SessionIdentity,
        client_data: String,
        startup: Vec<u8>,
        reply: SyncSender<Vec<RelayPack>>,
    },
    Batch {
        direction: Direction,
        body: Vec<u8>,
        reply: SyncSender<Option<Vec<u8>>>,
    },
    Tick {
        reply: SyncSender<RelayOutput>,
    },
    Stop,
}

/// A session's relay. The instance lives on its own thread, so WASI never runs on the session's
/// async runtime and every call is serialized; a failed instance passes everything through.
struct PacketModRelay {
    requests: Sender<Request>,
}

impl PacketModRelay {
    /// Starts the instance thread; it instantiates the packet mod when the session starts.
    fn spawn(loader: PacketModLoader) -> Self {
        let (requests, receive) = mpsc::channel();
        if let Err(error) = thread::Builder::new()
            .name("packet-mod".into())
            .spawn(move || serve(&loader, receive))
        {
            tracing::warn!(%error, "packet mod thread unavailable; packets pass through");
        }
        Self { requests }
    }

    /// Sends one request and waits for its answer; a stopped thread answers with `fallback`.
    fn ask<T>(&self, request: impl FnOnce(SyncSender<T>) -> Request, fallback: T) -> T {
        let (reply, answer) = mpsc::sync_channel(1);
        if self.requests.send(request(reply)).is_err() {
            return fallback;
        }
        answer.recv().unwrap_or(fallback)
    }

    fn batch(&self, direction: Direction, body: &[u8]) -> Option<Vec<u8>> {
        let body = body.to_vec();
        self.ask(
            |reply| Request::Batch {
                direction,
                body,
                reply,
            },
            None,
        )
    }
}

impl PacketRelay for PacketModRelay {
    fn start(
        &self,
        identity: &protocol::session_wire::SessionIdentity,
        client_data: &str,
        startup: &[u8],
    ) -> Vec<RelayPack> {
        let identity = SessionIdentity {
            display_name: identity.display_name.clone(),
            xuid: identity.xuid.clone(),
            uuid: identity.uuid.clone(),
        };
        let (client_data, startup) = (client_data.to_owned(), startup.to_vec());
        self.ask(
            |reply| Request::Start {
                identity,
                client_data,
                startup,
                reply,
            },
            Vec::new(),
        )
    }

    fn inbound(&self, body: &[u8]) -> Option<Vec<u8>> {
        self.batch(Direction::FromServer, body)
    }

    fn outbound(&self, body: &[u8]) -> Option<Vec<u8>> {
        self.batch(Direction::FromClient, body)
    }

    fn tick(&self) -> RelayOutput {
        self.ask(|reply| Request::Tick { reply }, RelayOutput::default())
    }

    fn stop(&self) {
        let _ = self.requests.send(Request::Stop);
    }
}

/// Answers the session's requests in order until it stops or goes away.
fn serve(loader: &PacketModLoader, requests: Receiver<Request>) {
    let mut instance: Option<PacketMod> = None;
    for request in requests {
        match request {
            Request::Start {
                identity,
                client_data,
                startup,
                reply,
            } => {
                instance = start(loader);
                let packs = instance.as_mut().map_or_else(Vec::new, |instance| {
                    instance
                        .start(&identity, &client_data, &startup)
                        .unwrap_or_default()
                });
                let packs = packs
                    .into_iter()
                    .map(|pack| RelayPack {
                        archive: pack.archive,
                        above_server: pack.above_server,
                    })
                    .collect();
                let _ = reply.send(packs);
            }
            Request::Batch {
                direction,
                body,
                reply,
            } => {
                let replaced = instance
                    .as_mut()
                    .and_then(|instance| instance.on_batch(direction, &body).ok().flatten());
                let _ = reply.send(replaced);
            }
            Request::Tick { reply } => {
                let output = instance
                    .as_mut()
                    .and_then(|instance| instance.tick().ok())
                    .map_or_else(RelayOutput::default, |outbound| RelayOutput {
                        to_client: outbound.to_client,
                        to_server: outbound.to_server,
                    });
                let _ = reply.send(output);
            }
            Request::Stop => break,
        }
    }
    if let Some(instance) = instance.as_mut() {
        instance.stop();
    }
}

/// Instantiates the selected packet mod for this session, or explains why the session runs
/// without it.
fn start(loader: &PacketModLoader) -> Option<PacketMod> {
    let Some(compiled) = loader.compiled(PACKET_MOD_READY_WAIT) else {
        tracing::warn!("packet mod not ready; this session runs without it");
        return None;
    };
    compiled
        .instantiate()
        .inspect_err(|error| tracing::warn!(error = format!("{error:#}"), "packet mod disabled"))
        .ok()
}

#[cfg(test)]
mod tests;
