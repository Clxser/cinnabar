//! Worker-owned process supervision keeps cancellation effective during blocked pipe IO.

use std::{
    process::{Child, ChildStdin, ChildStdout, Command, ExitStatus},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use crossbeam_channel::{Receiver, bounded};

use crate::export::Cancellation;

pub(crate) struct MediaChild {
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
    aborted: Arc<AtomicBool>,
    result: Receiver<Result<ExitStatus, String>>,
    watcher: Option<JoinHandle<()>>,
}

/// Even a failed supervisor-thread spawn drops this guard and reaps its own process.
struct OwnedChild(Child);

impl OwnedChild {
    /// Calling through the wrapper keeps its reap-on-drop guard inside a failed thread-spawn closure.
    fn supervise(
        &mut self,
        cancellation: &Cancellation,
        stop: &AtomicBool,
    ) -> Result<ExitStatus, String> {
        loop {
            if stop.load(Ordering::Acquire) || cancellation.cancelled() {
                let _ = self.0.kill();
                let _ = self.0.wait();
                return Err("Export cancelled.".into());
            }
            match self.0.try_wait() {
                Ok(Some(status)) => return Ok(status),
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(error) => return Err(error.to_string()),
            }
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
    /// Called only on an IO worker. No shell interprets argv and no background console is opened.
    pub fn start(mut command: Command, cancellation: Cancellation) -> Result<Self, String> {
        cancellation.check()?;
        super::hide_console(&mut command);
        let mut child = OwnedChild(command.spawn().map_err(|error| error.to_string())?);
        let stdin = child.0.stdin.take();
        let stdout = child.0.stdout.take();
        let aborted = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&aborted);
        let (completed, result) = bounded(1);
        let watcher = std::thread::Builder::new()
            .name("studio-media-process".into())
            .spawn(move || {
                let result = child.supervise(&cancellation, &stop);
                let _ = completed.send(result);
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            stdin,
            stdout,
            aborted,
            result,
            watcher: Some(watcher),
        })
    }

    /// Closing input precedes waiting so normal FFmpeg flushing can finish. Only the worker waits.
    pub fn finish(mut self) -> Result<(), String> {
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
