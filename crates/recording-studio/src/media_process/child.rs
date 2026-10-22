//! Worker-owned process supervision keeps cancellation effective during blocked pipe IO.

use std::{
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, ExitStatus},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, bounded};

use crate::export::Cancellation;

mod progress;

pub(crate) struct MediaChild {
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
    aborted: Arc<AtomicBool>,
    result: Receiver<Result<ExitStatus, String>>,
    watcher: Option<JoinHandle<()>>,
    watchdog: Arc<Mutex<Watchdog>>,
    activity: Arc<AtomicU64>,
    output: Option<(PathBuf, u64)>,
}

/// Only blocking operations arm this watchdog; a paused preview may retain idle decoders indefinitely.
pub(crate) struct IoDeadline(Arc<Mutex<Watchdog>>);

impl Drop for IoDeadline {
    fn drop(&mut self) {
        self.0.lock().unwrap_or_else(|error| error.into_inner()).io = None;
    }
}

/// An idle or completed operation has no deadline, including while Studio transport is paused.
fn io_expired(deadline: Option<Instant>, now: Instant) -> bool {
    deadline.is_some_and(|deadline| now >= deadline)
}

#[derive(Default)]
struct Watchdog {
    io: Option<Instant>,
    finish: Option<progress::Finalization>,
}

impl Watchdog {
    /// Process progress refreshes finalization only; idle retained decoders have no armed operation.
    fn expired(&mut self, now: Instant, activity: u64) -> bool {
        io_expired(self.io, now)
            || self
                .finish
                .as_mut()
                .is_some_and(|finish| finish.expired(now, activity))
    }
}

/// Even a failed supervisor-thread spawn drops this guard and reaps its own process.
struct OwnedChild(Child);

impl OwnedChild {
    /// Calling through the wrapper keeps its reap-on-drop guard inside a failed thread-spawn closure.
    fn supervise(
        &mut self,
        cancellation: &Cancellation,
        stop: &AtomicBool,
        timeout: Option<Duration>,
        watchdog: &Mutex<Watchdog>,
        activity: &AtomicU64,
    ) -> Result<ExitStatus, String> {
        let started = std::time::Instant::now();
        loop {
            if stop.load(Ordering::Acquire) || cancellation.cancelled() {
                let _ = self.0.kill();
                let _ = self.0.wait();
                return Err("Export cancelled.".into());
            }
            match self.0.try_wait() {
                Ok(Some(status)) => return Ok(status),
                Ok(None) => {}
                Err(error) => return Err(error.to_string()),
            }
            if watchdog
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .expired(Instant::now(), activity.load(Ordering::Acquire))
            {
                return Err("The media operation stopped making progress before its inactivity limit. Check the source file, encoder and available storage, then retry.".into());
            }
            if timeout.is_some_and(|limit| started.elapsed() >= limit) {
                return Err("The media process exceeded its time limit.".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl MediaChild {
    /// Metered binary output supplies progress without mixing FFmpeg text into its media pipe.
    pub(crate) fn activity_counter(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.activity)
    }
    /// Called only on an IO worker. No shell interprets argv and no background console is opened.
    pub fn start(command: Command, cancellation: Cancellation) -> Result<Self, String> {
        Self::start_inner(command, cancellation, None, None)
    }

    /// Export progress uses a private bounded pipe; its output path also bounds silent index relocation.
    pub fn start_output(
        mut command: Command,
        cancellation: Cancellation,
        output: &Path,
        relocation_bytes: u64,
    ) -> Result<Self, String> {
        command
            .args(["-progress", "pipe:1", "-stats_period", "1", "-nostats"])
            .stdout(std::process::Stdio::piped());
        Self::start_inner(
            command,
            cancellation,
            None,
            Some((output.to_owned(), relocation_bytes)),
        )
    }

    /// Bounded probes and recovery jobs reap their process when either cancellation or timeout wins.
    pub fn start_timed(
        command: Command,
        cancellation: Cancellation,
        timeout: Duration,
    ) -> Result<Self, String> {
        Self::start_inner(command, cancellation, Some(timeout), None)
    }

    /// Transfers process ownership before exposing its bounded pipes to an IO worker.
    fn start_inner(
        mut command: Command,
        cancellation: Cancellation,
        timeout: Option<Duration>,
        output: Option<(PathBuf, u64)>,
    ) -> Result<Self, String> {
        cancellation.check()?;
        super::hide_console(&mut command);
        let mut child = OwnedChild(command.spawn().map_err(|error| error.to_string())?);
        let stdin = child.0.stdin.take();
        let mut stdout = child.0.stdout.take();
        let aborted = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&aborted);
        let (completed, result) = bounded(1);
        let watchdog = Arc::new(Mutex::new(Watchdog::default()));
        let watched = Arc::clone(&watchdog);
        let activity = Arc::new(AtomicU64::new(0));
        let watched_activity = Arc::clone(&activity);
        let progress = if output.is_some() {
            Some(progress::spawn(
                stdout
                    .take()
                    .ok_or("Export progress output is unavailable.")?,
                Arc::clone(&activity),
            )?)
        } else {
            None
        };
        let watcher = std::thread::Builder::new()
            .name("studio-media-process".into())
            .spawn(move || {
                let result =
                    child.supervise(&cancellation, &stop, timeout, &watched, &watched_activity);
                // Reap first so an error or cancellation also closes the progress reader's pipe.
                drop(child);
                if let Some(progress) = progress {
                    let _ = progress.join();
                }
                let _ = completed.send(result);
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            stdin,
            stdout,
            aborted,
            result,
            watcher: Some(watcher),
            watchdog,
            activity,
            output,
        })
    }

    /// A stalled read/write is killed by its existing supervisor so cancellation and IO cannot hang.
    pub(crate) fn guard_io(&self) -> IoDeadline {
        self.watchdog
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .io = Some(Instant::now() + Duration::from_secs(60));
        IoDeadline(Arc::clone(&self.watchdog))
    }

    /// Closing input precedes waiting so normal FFmpeg flushing can finish. Only the worker waits.
    pub fn finish(mut self) -> Result<(), String> {
        let relocation_bytes = self.output.as_ref().map_or(0, |(path, minimum)| {
            std::fs::metadata(path).map_or(*minimum, |metadata| metadata.len().max(*minimum))
        });
        {
            let mut watchdog = self
                .watchdog
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            watchdog.io = None;
            watchdog.finish = Some(progress::Finalization::new(
                Instant::now(),
                self.activity.load(Ordering::Acquire),
                relocation_bytes,
            ));
        }
        self.stdin.take();
        self.stdout.take();
        let result = self
            .result
            .recv()
            .map_err(|_| "The media process supervisor stopped.".to_owned())?;
        if let Some(watcher) = self.watcher.take() {
            watcher
                .join()
                .map_err(|_| "The media process supervisor panicked.".to_owned())?;
        }
        let status = result?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("FFmpeg exited with {status}."))
        }
    }

    /// A completed preview clip lets its existing supervisor reap asynchronously without blocking the mixer.
    pub(crate) fn retire(mut self) {
        self.aborted.store(true, Ordering::Release);
        self.stdin.take();
        self.stdout.take();
        self.watcher.take();
    }
}

impl Drop for MediaChild {
    /// Export teardown is worker-local; cancelling the public task itself never blocks the UI thread.
    fn drop(&mut self) {
        self.aborted.store(true, Ordering::Release);
        self.stdin.take();
        self.stdout.take();
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_timeout_does_not_expire_an_idle_paused_decoder() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(60);
        assert!(!io_expired(None, now + Duration::from_secs(86_400)));
        assert!(!io_expired(Some(deadline), now));
        assert!(io_expired(Some(deadline), deadline));
        let state = Arc::new(Mutex::new(Watchdog {
            io: Some(deadline),
            finish: None,
        }));
        let guard = IoDeadline(Arc::clone(&state));
        drop(guard);
        assert!(!state.lock().unwrap().expired(deadline, 0));
    }
}
