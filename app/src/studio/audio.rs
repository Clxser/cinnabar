//! Disk writes consume a bounded passive audio tap off both the game and hardware callback threads.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, Thread},
    time::{Duration, Instant},
};

use client_audio::named_audio::{CAPTURE_CHANNELS, LiveAudioTap};
use crossbeam_channel::{Receiver, bounded};
use recording_studio::{MediaTime, time::MICROS_PER_SECOND, wav::WavWriter};

struct Control {
    stop: AtomicBool,
    samples: AtomicU64,
}

pub(super) struct AudioWorker {
    control: Arc<Control>,
    thread: Thread,
    completed: Receiver<Result<(PathBuf, u64), String>>,
    rate: u32,
}

impl AudioWorker {
    /// A completion before an explicit stop indicates capture failed while video was still running.
    pub fn failed(&self) -> bool {
        !self.control.stop.load(Ordering::Acquire) && !self.completed.is_empty()
    }

    /// Moves the mixed stream to its own worker before any WAV file is opened.
    pub fn start(tap: LiveAudioTap, path: PathBuf) -> Result<Self, String> {
        let rate = tap.sample_rate();
        let control = Arc::new(Control {
            stop: AtomicBool::new(false),
            samples: AtomicU64::new(u64::MAX),
        });
        let worker = Arc::clone(&control);
        let (done, completed) = bounded(1);
        let thread = thread::Builder::new()
            .name("studio-live-audio".into())
            .spawn(move || {
                let result = run(tap, path, &worker);
                let _ = done.send(result);
            })
            .map_err(|error| error.to_string())?;
        let thread = thread.thread().clone();
        Ok(Self {
            control,
            thread,
            completed,
            rate,
        })
    }

    /// Freezes the sample duration immediately; final file flushes remain on the worker.
    pub fn stop(&self, duration: MediaTime) {
        let samples = (u128::from(duration.0) * u128::from(self.rate)
            / u128::from(MICROS_PER_SECOND))
            * u128::from(CAPTURE_CHANNELS);
        self.control
            .samples
            .store(samples.min(u128::from(u64::MAX)) as u64, Ordering::Release);
        self.control.stop.store(true, Ordering::Release);
        self.thread.unpark();
    }

    /// Only the recording finalizer may wait for disk completion.
    pub fn finish(self) -> Result<(PathBuf, u64), String> {
        self.completed
            .recv()
            .map_err(|_| "The audio recording worker stopped unexpectedly.".to_owned())?
    }
}

impl Drop for AudioWorker {
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::Release);
        self.thread.unpark();
    }
}

/// Uses block offsets to keep a full disk queue from shifting subsequent audio out of sync.
fn run(tap: LiveAudioTap, path: PathBuf, control: &Control) -> Result<(PathBuf, u64), String> {
    let mut wav = WavWriter::create_rf64(&path, CAPTURE_CHANNELS, tap.sample_rate())
        .map_err(|error| error.to_string())?;
    let mut written = 0_u64;
    let mut stopping = None;
    loop {
        if control.stop.load(Ordering::Acquire) && stopping.is_none() {
            tap.stop();
            stopping = Some(Instant::now());
        }
        let maximum = control.samples.load(Ordering::Acquire);
        while let Some(block) = tap.pop() {
            let start = block.offset.min(maximum);
            if start > written {
                wav.write(std::iter::repeat_n(0.0, (start - written) as usize))
                    .map_err(|error| error.to_string())?;
                written = start;
            }
            let skipped = written.saturating_sub(block.offset).min(block.len as u64) as usize;
            let count = (block.len - skipped).min(maximum.saturating_sub(written) as usize);
            wav.write(block.samples[skipped..skipped + count].iter().copied())
                .map_err(|error| error.to_string())?;
            written += count as u64;
        }
        if stopping
            .is_some_and(|started| tap.drained() || started.elapsed() >= Duration::from_millis(250))
        {
            let maximum = if maximum == u64::MAX {
                written
            } else {
                maximum
            };
            wav.trim_samples(maximum)
                .map_err(|error| error.to_string())?;
            if maximum > written {
                wav.write(std::iter::repeat_n(0.0, (maximum - written) as usize))
                    .map_err(|error| error.to_string())?;
            }
            let dropped = tap.dropped_samples();
            return wav
                .finish()
                .map(|path| (path, dropped))
                .map_err(|error| error.to_string());
        }
        thread::park_timeout(Duration::from_millis(8));
    }
}
