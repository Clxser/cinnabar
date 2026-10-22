//! One owned source verifier keeps rapid replacement requests from spawning concurrent disk scans.

use super::*;
use crossbeam_channel::{Receiver, Sender};

struct Work {
    request: TargetCoverageRequest,
    reader: TargetCoverageReader,
    cancellation: Cancellation,
    result: Sender<TargetCoverageResult>,
}

pub struct TargetVerifier {
    sender: Sender<Work>,
    waiting: Receiver<Work>,
}

impl TargetVerifier {
    /// Starts one worker lazily; it exits after its owner disconnects and an active cancelled scan returns.
    pub fn start() -> Result<Self, String> {
        let (sender, waiting) = crossbeam_channel::bounded::<Work>(1);
        let receiver = waiting.clone();
        std::thread::Builder::new()
            .name("studio-camera-coverage".into())
            .spawn(move || {
                while let Ok(work) = receiver.recv() {
                    let result = work
                        .cancellation
                        .check()
                        .and_then(|()| (work.reader)(&work.request, &work.cancellation));
                    let _ = work.result.try_send(result);
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self { sender, waiting })
    }

    /// Replaces queued work without waiting; callers cancel the superseded active request's token.
    pub fn submit(
        &self,
        reader: TargetCoverageReader,
        request: TargetCoverageRequest,
        cancellation: Cancellation,
    ) -> Result<Receiver<TargetCoverageResult>, String> {
        while let Ok(old) = self.waiting.try_recv() {
            old.cancellation.cancel();
        }
        let (result, receiver) = crossbeam_channel::bounded(1);
        self.sender
            .try_send(Work {
                reader,
                request,
                cancellation,
                result,
            })
            .map_err(|_| "The camera verifier is unavailable. Retry the suggestion.".to_string())?;
        Ok(receiver)
    }
}

impl Drop for TargetVerifier {
    /// Queued requests are abandoned without joining a disk worker on the editor thread.
    fn drop(&mut self) {
        while let Ok(work) = self.waiting.try_recv() {
            work.cancellation.cancel();
        }
    }
}

#[cfg(test)]
mod tests;
