//! One cancellable mixer prepares a bounded PCM queue; decoding and process retirement stay off the UI.

use client_audio::preview::{
    PREVIEW_BLOCK_FRAMES, PREVIEW_BLOCK_MILLIS, PreviewBlock, PreviewOutput,
};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use recording_studio::{
    audio::{MonitorClock, MonitorMixer, PreviewRequest},
    export::Cancellation,
};
use std::time::Duration;

struct Job {
    request: PreviewRequest,
    output: PreviewOutput,
    generation: u64,
    token: u64,
    cancellation: Cancellation,
}

pub(super) struct Worker {
    submit: Sender<Job>,
    pending: Receiver<Job>,
    errors: Receiver<(u64, String)>,
    cancellation: Option<Cancellation>,
    tokens: JobTokens,
}

#[derive(Default)]
struct JobTokens(u64);

impl JobTokens {
    /// Tokens belong to the worker, so different output lanes cannot reuse an error identity.
    fn next(&mut self) -> Result<u64, String> {
        self.0 = self
            .0
            .checked_add(1)
            .ok_or("The Studio audio job identity is exhausted.")?;
        Ok(self.0)
    }
}

impl Worker {
    /// The worker sleeps when idle; retaining its sender does not retain any old decoder or media file.
    pub fn start() -> Result<Self, String> {
        let (submit, pending) = bounded::<Job>(1);
        let (failed, errors) = bounded(4);
        let jobs = pending.clone();
        let evict_errors = errors.clone();
        std::thread::Builder::new()
            .name("studio-preview-audio".into())
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&job)));
                    let error = match result {
                        Ok(Err(error)) => Some(error),
                        Err(_) => Some("The Studio audio worker stopped unexpectedly.".into()),
                        _ => None,
                    };
                    if let Some(error) = error.filter(|_| !job.cancellation.cancelled()) {
                        let value = (job.token, error.chars().take(1024).collect());
                        if let Err(TrySendError::Full(value)) = failed.try_send(value) {
                            let _ = evict_errors.try_recv();
                            let _ = failed.try_send(value);
                        }
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            submit,
            pending,
            errors,
            cancellation: None,
            tokens: JobTokens::default(),
        })
    }

    /// A new document/edit/seek invalidates blocked decoder reads as well as queued PCM.
    pub fn submit(
        &mut self,
        request: PreviewRequest,
        output: PreviewOutput,
        generation: u64,
    ) -> Result<u64, String> {
        self.cancel();
        let token = self.tokens.next()?;
        let cancellation = Cancellation::default();
        self.cancellation = Some(cancellation.clone());
        let job = Job {
            request,
            output,
            generation,
            token,
            cancellation,
        };
        match self.submit.try_send(job) {
            Ok(()) => Ok(token),
            Err(TrySendError::Full(job)) => {
                let _ = self.pending.try_recv();
                self.submit
                    .try_send(job)
                    .map(|()| token)
                    .map_err(|_| "The Studio audio worker is unavailable.".into())
            }
            Err(TrySendError::Disconnected(_)) => {
                Err("The Studio audio worker is unavailable.".into())
            }
        }
    }

    /// Cancellation is atomic and never joins a decoder or thread on the UI thread.
    pub fn cancel(&mut self) {
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
    }

    /// Only a failure belonging to this exact job can change playback state.
    pub fn error(&self, token: u64) -> Option<String> {
        take_error(&self.errors, token)
    }
}

/// Polling is bounded by the channel capacity and ignores every retired lane's completion.
fn take_error(errors: &Receiver<(u64, String)>, token: u64) -> Option<String> {
    let mut error = None;
    for _ in 0..4 {
        match errors.try_recv() {
            Ok((id, message)) if id == token => error = Some(message),
            Ok(_) => {}
            Err(_) => break,
        }
    }
    error
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// The queue and fixed block own all buffering; decoder work cannot accumulate behind transport time.
fn run(job: &Job) -> Result<(), String> {
    let rate = job.output.sample_rate();
    let clock = MonitorClock::new(
        job.request.preview.at,
        job.request.duration,
        job.request.speed.rate(),
        rate,
    )?;
    let mut cursor = 0;
    let mut mixer = MonitorMixer::new(
        job.request.preview.project.clone(),
        &job.request.preview.source_base,
        clock,
        job.cancellation.clone(),
    )?;
    let end = mixer.end_frame();
    let block_frames = (u64::from(rate) * u64::from(PREVIEW_BLOCK_MILLIS))
        .div_ceil(1000)
        .clamp(1, PREVIEW_BLOCK_FRAMES as u64);
    while !job.cancellation.cancelled() && job.output.generation() == job.generation {
        if cursor >= end || job.output.full() {
            std::thread::park_timeout(Duration::from_millis(3));
            continue;
        }
        let len = (end - cursor).min(block_frames) as usize;
        let mut block = PreviewBlock {
            generation: job.generation,
            start: cursor,
            len,
            samples: [[0.0; 2]; PREVIEW_BLOCK_FRAMES],
        };
        mixer.read(&mut block.samples[..len], cursor, &job.cancellation)?;
        loop {
            job.cancellation.check()?;
            if job.output.generation() != job.generation {
                return Ok(());
            }
            match job.output.push(block) {
                Ok(()) => break,
                Err(retained) => {
                    block = retained;
                    std::thread::park_timeout(Duration::from_millis(3));
                }
            }
        }
        cursor += len as u64;
        if cursor == end {
            mixer.retire_decoders();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_lanes_do_not_reuse_the_previous_jobs_error_identity() {
        let mut tokens = JobTokens::default();
        let first = tokens.next().unwrap();
        let replacement = tokens.next().unwrap();
        assert_ne!(first, replacement);
        let (send, errors) = bounded(4);
        send.try_send((first, "retired device".into())).unwrap();
        assert_eq!(take_error(&errors, replacement), None);
        send.try_send((first, "late retired device".into()))
            .unwrap();
        send.try_send((replacement, "current device".into()))
            .unwrap();
        assert_eq!(
            take_error(&errors, replacement).as_deref(),
            Some("current device")
        );
        tokens.0 = u64::MAX;
        assert!(tokens.next().is_err());
    }
}
