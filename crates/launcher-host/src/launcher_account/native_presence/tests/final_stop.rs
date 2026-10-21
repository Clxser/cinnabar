//! Final process cancellation at the original tracked account event owner.

use super::*;
use bridge::tasks::{NativeExecution, StopAuthority};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc as sync,
};
use tokio_util::task::TaskTracker;

#[derive(Debug, PartialEq, Eq)]
enum Observation {
    Dispatched,
    StillHeld,
    Dropped,
    Returned,
}

/// A dispatched service future that completes only through its explicit fixture release.
pub(in crate::launcher_account) struct PendingPresence {
    observations: sync::Sender<Observation>,
    probes: Mutex<Option<mpsc::Receiver<()>>>,
    release: Mutex<Option<oneshot::Receiver<()>>>,
    dropped: AtomicBool,
    removals: AtomicUsize,
}

struct PendingUpdate<'a>(&'a PendingPresence);

impl Drop for PendingUpdate<'_> {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::Release);
        let _ = self.0.observations.send(Observation::Dropped);
    }
}

impl Service for PendingPresence {
    type Scope = ();

    async fn bind(&self) -> io::Result<()> {
        Ok(())
    }

    async fn update(
        &self,
        _: &(),
        _: &bridge::XboxPresenceState,
        dispatched: &mut bool,
    ) -> io::Result<Option<Duration>> {
        let _actual_future = PendingUpdate(self);
        let mut probes = self.probes.lock().unwrap().take().unwrap();
        let mut release = self.release.lock().unwrap().take().unwrap();
        *dispatched = true;
        self.observations.send(Observation::Dispatched).unwrap();
        loop {
            tokio::select! {
                _ = &mut release => return Ok(None),
                probe = probes.recv() => {
                    if probe.is_some() {
                        self.observations.send(Observation::StillHeld).unwrap();
                    }
                }
            }
        }
    }

    async fn remove(&self, _: &()) -> io::Result<()> {
        self.removals.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

struct Harness {
    _runtime: tokio::runtime::Runtime,
    execution: NativeExecution,
    stop: StopAuthority,
    tasks: TaskTracker,
    lifetime: TitlePresenceLifetime,
    service: Arc<PendingPresence>,
    observations: sync::Receiver<Observation>,
    probes: mpsc::Sender<()>,
    release: Option<oneshot::Sender<()>>,
}

impl Harness {
    /// Pauses service budgets so rendezvous, rather than elapsed time, decides every outcome.
    fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap();
        let tasks = TaskTracker::new();
        let (execution, stop) =
            NativeExecution::for_process(runtime.handle().clone(), tasks.clone());
        let (observed, observations) = sync::channel();
        let (probes, received) = mpsc::channel(1);
        let (release, released) = oneshot::channel();
        Self {
            _runtime: runtime,
            execution,
            stop,
            tasks,
            lifetime: TitlePresenceLifetime::new(),
            service: Arc::new(PendingPresence {
                observations: observed,
                probes: Mutex::new(Some(received)),
                release: Mutex::new(Some(released)),
                dropped: AtomicBool::new(false),
                removals: AtomicUsize::new(0),
            }),
            observations,
            probes,
            release: Some(release),
        }
    }

    /// Starts the real event/publisher/WorkerGuard body inside the production OS-worker tracker.
    fn start(&self) -> std::thread::JoinHandle<()> {
        let shared = Mutex::new(Snapshot {
            pending_presence_probe: Some(Arc::clone(&self.service)),
            ..Snapshot::default()
        });
        let (requests, received) = bounded(1);
        let lifetime = self.lifetime.clone();
        let execution = self.execution.clone();
        let observations = self.service.observations.clone();
        std::thread::spawn(crate::worker::tracked(
            move || {
                let _requests = requests;
                events::poll(&shared, &received, &lifetime, execution.runtime());
                observations.send(Observation::Returned).unwrap();
            },
            &self.tasks,
        ))
    }

    /// Bounds only failed fixture rendezvous; service clocks remain paused throughout.
    fn observed(&self) -> Observation {
        self.observations
            .recv_timeout(test_time::DEFAULT_TIMEOUT)
            .expect("account event owner did not reach its fixture rendezvous")
    }

    /// Releases a RED witness before asserting its outcome, without leaking the original worker.
    fn finish(&mut self, worker: std::thread::JoinHandle<()>) {
        self.lifetime.retire();
        let _ = self.release.take().unwrap().send(());
        worker.join().unwrap();
        self.tasks.close();
        let mut wait = std::pin::pin!(self.tasks.wait());
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(std::future::Future::poll(wait.as_mut(), &mut context).is_ready());
        assert!(self.tasks.is_empty());
        assert!(self.lifetime.worker_finished());
    }
}

#[test]
fn ordinary_retirement_keeps_the_dispatched_presence_until_completion() {
    let mut harness = Harness::new();
    let worker = harness.start();
    assert_eq!(harness.observed(), Observation::Dispatched);
    harness.lifetime.retire();
    harness.probes.try_send(()).unwrap();
    assert_eq!(harness.observed(), Observation::StillHeld);
    assert!(!harness.service.dropped.load(Ordering::Acquire));
    assert_eq!(harness.lifetime.retirement(), None);
    assert!(!harness.lifetime.worker_finished());
    assert!(!harness.tasks.is_empty());
    harness.finish(worker);
    assert_eq!(harness.service.removals.load(Ordering::Acquire), 1);
    assert_eq!(
        harness.lifetime.retirement(),
        Some(TitlePresenceRetirement::Complete)
    );
}

#[test]
fn final_process_stop_drops_dispatched_presence_and_retires_the_actual_worker() {
    let mut harness = Harness::new();
    let worker = harness.start();
    assert_eq!(harness.observed(), Observation::Dispatched);
    harness.lifetime.retire();
    harness.tasks.close();
    harness.stop.stop();
    let _ = harness.probes.try_send(());
    let after_stop = harness.observed();
    let dropped_before_release = harness.service.dropped.load(Ordering::Acquire);
    harness.finish(worker);
    assert_eq!(after_stop, Observation::Dropped);
    assert!(dropped_before_release);
    assert_eq!(harness.service.removals.load(Ordering::Acquire), 0);
    assert_eq!(
        harness.lifetime.retirement(),
        Some(TitlePresenceRetirement::Unconfirmed)
    );
}

#[test]
fn final_process_stop_is_sticky_before_the_account_worker_starts() {
    let mut harness = Harness::new();
    harness.tasks.close();
    harness.stop.stop();
    let worker = harness.start();
    let first = harness.observed();
    harness.finish(worker);
    assert_eq!(first, Observation::Returned);
    assert!(!harness.service.dropped.load(Ordering::Acquire));
    assert_eq!(harness.service.removals.load(Ordering::Acquire), 0);
    assert_eq!(
        harness.lifetime.retirement(),
        Some(TitlePresenceRetirement::Unconfirmed)
    );
}
