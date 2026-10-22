//! An exclusive PCM lane drains live game sources without making them audible in an editor.

use crossbeam_queue::ArrayQueue;
use rodio::Source;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::Duration,
};

pub const PREVIEW_BLOCK_FRAMES: usize = 4096;
pub const PREVIEW_BLOCK_MILLIS: u32 = 10;
const QUEUED_BLOCKS: usize = 8;
pub const PREVIEW_CHANNELS: u16 = 2;

pub struct PreviewBlock {
    pub generation: u64,
    pub start: u64,
    pub len: usize,
    pub samples: [[f32; 2]; PREVIEW_BLOCK_FRAMES],
}

struct Shared {
    queue: ArrayQueue<PreviewBlock>,
    generation: AtomicU64,
    rate: AtomicU32,
    exclusive: AtomicBool,
    playing: AtomicBool,
    pause: AtomicU64,
    paused: AtomicU64,
    position: AtomicU64,
    position_generation: AtomicU64,
    start: AtomicU64,
    ceiling: AtomicU64,
    end: AtomicU64,
    buffered: AtomicU64,
    buffered_generation: AtomicU64,
    waiting: AtomicBool,
    underruns: AtomicU64,
    gain: AtomicU32,
}

#[derive(Clone)]
pub struct PreviewOutput(Arc<Shared>);

impl PreviewOutput {
    /// Allocation and queue ownership stay outside the device callback.
    pub(crate) fn new(rate: u32) -> Self {
        Self(Arc::new(Shared {
            queue: ArrayQueue::new(QUEUED_BLOCKS),
            generation: AtomicU64::new(0),
            rate: AtomicU32::new(rate),
            exclusive: AtomicBool::new(false),
            playing: AtomicBool::new(false),
            pause: AtomicU64::new(0),
            paused: AtomicU64::new(u64::MAX),
            position: AtomicU64::new(0),
            position_generation: AtomicU64::new(0),
            start: AtomicU64::new(0),
            ceiling: AtomicU64::new(0),
            end: AtomicU64::new(0),
            buffered: AtomicU64::new(0),
            buffered_generation: AtomicU64::new(0),
            waiting: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
            gain: AtomicU32::new(1.0_f32.to_bits()),
        }))
    }

    /// The hardware's negotiated rate is set before submitting its source.
    pub(crate) fn set_sample_rate(&self, rate: u32) {
        self.0.rate.store(rate, Ordering::Release);
    }
    pub fn sample_rate(&self) -> u32 {
        self.0.rate.load(Ordering::Acquire)
    }
    pub fn generation(&self) -> u64 {
        self.0.generation.load(Ordering::Acquire)
    }
    pub fn position(&self) -> u64 {
        let generation = self.generation();
        if self.0.position_generation.load(Ordering::Acquire) == generation {
            self.0.position.load(Ordering::Acquire)
        } else {
            self.0.start.load(Ordering::Acquire)
        }
    }
    /// The callback acknowledges Pause only after any already-running stereo frame has finished.
    pub fn paused_position(&self) -> Option<u64> {
        (!self.0.playing.load(Ordering::Acquire)
            && self.0.paused.load(Ordering::Acquire) == self.0.pause.load(Ordering::Acquire)
            && self.0.position_generation.load(Ordering::Acquire) == self.generation())
        .then(|| self.0.position.load(Ordering::Acquire))
    }
    pub fn buffered_until(&self) -> u64 {
        if self.0.buffered_generation.load(Ordering::Acquire) == self.generation() {
            self.0.buffered.load(Ordering::Acquire)
        } else {
            self.0.start.load(Ordering::Acquire)
        }
    }
    pub fn waiting(&self) -> bool {
        self.0.waiting.load(Ordering::Acquire)
    }
    pub fn underruns(&self) -> u64 {
        self.0.underruns.load(Ordering::Acquire)
    }
    pub fn full(&self) -> bool {
        self.0.queue.is_full()
    }
    /// Replacing a hardware device also replaces its lane, even when its sample rate is unchanged.
    pub fn same_output(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Editor ownership also mutes already-playing gameplay and UI sources; capture remains upstream.
    pub fn set_exclusive(&self, exclusive: bool) {
        self.0.exclusive.store(exclusive, Ordering::Release);
    }
    /// Monitoring respects application master volume without changing saved clip gain or export samples.
    pub fn set_gain(&self, gain: f32) {
        if gain.is_finite() {
            self.0
                .gain
                .store(gain.clamp(0.0, 1.0).to_bits(), Ordering::Release);
        }
    }

    /// An epoch change retires queued and callback-held PCM without waiting for a worker or device.
    pub fn reset(&self, start: u64, end: u64) -> u64 {
        self.set_playing(false, start);
        self.0.start.store(start, Ordering::Release);
        self.0.end.store(end, Ordering::Release);
        self.0.waiting.store(false, Ordering::Release);
        self.0.underruns.store(0, Ordering::Release);
        for _ in 0..QUEUED_BLOCKS {
            if self.0.queue.pop().is_none() {
                break;
            }
        }
        self.0
            .generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
    }

    /// Displayed-frame backpressure limits the audio clock as well as the timeline clock.
    pub fn set_playing(&self, playing: bool, ceiling: u64) {
        self.0.ceiling.store(ceiling, Ordering::Release);
        if self.0.playing.swap(playing, Ordering::AcqRel) && !playing {
            self.0.pause.fetch_add(1, Ordering::AcqRel);
        }
    }

    /// Only prepared finite stereo blocks cross into the callback; a full queue preserves ownership.
    pub fn push(&self, block: PreviewBlock) -> Result<(), PreviewBlock> {
        if block.generation != self.generation()
            || block.len == 0
            || block.len > PREVIEW_BLOCK_FRAMES
            || block.start.checked_add(block.len as u64).is_none()
            || block.samples[..block.len]
                .iter()
                .flatten()
                .any(|sample| !sample.is_finite())
        {
            return Err(block);
        }
        let end = block.start.saturating_add(block.len as u64);
        let generation = block.generation;
        self.0.queue.push(block)?;
        self.0.buffered.store(end, Ordering::Release);
        self.0
            .buffered_generation
            .store(generation, Ordering::Release);
        Ok(())
    }
}

pub(crate) struct Routed<S> {
    game: S,
    output: PreviewOutput,
    current: Option<PreviewBlock>,
    frame: [f32; 2],
    channel: usize,
    generation: u64,
    starved: bool,
}

impl<S> Routed<S> {
    /// The game source includes the recording tap; preview PCM cannot enter that tap.
    pub fn new(game: S, output: PreviewOutput) -> Self {
        Self {
            game,
            output,
            current: None,
            frame: [0.0; 2],
            channel: 0,
            generation: u64::MAX,
            starved: false,
        }
    }

    /// No allocation, locking, media decode or IO occurs in the hardware callback.
    fn frame(&mut self) -> [f32; 2] {
        let shared = &self.output.0;
        let generation = self.output.generation();
        if generation != self.generation {
            self.current = None;
            self.generation = generation;
            self.starved = false;
            shared
                .position
                .store(shared.start.load(Ordering::Acquire), Ordering::Release);
            shared
                .position_generation
                .store(generation, Ordering::Release);
        }
        if !shared.exclusive.load(Ordering::Acquire) || !shared.playing.load(Ordering::Acquire) {
            shared.waiting.store(false, Ordering::Release);
            self.starved = false;
            if !shared.playing.load(Ordering::Acquire) {
                shared
                    .paused
                    .store(shared.pause.load(Ordering::Acquire), Ordering::Release);
            }
            return [0.0; 2];
        }
        let at = self.output.position();
        if at >= shared.end.load(Ordering::Acquire) {
            self.starved = false;
            return [0.0; 2];
        }
        if at >= shared.ceiling.load(Ordering::Acquire) {
            shared.waiting.store(true, Ordering::Release);
            self.starved = false;
            return [0.0; 2];
        }
        for _ in 0..=QUEUED_BLOCKS {
            if self.current.as_ref().is_some_and(|block| {
                block.generation != generation || at >= block.start.saturating_add(block.len as u64)
            }) {
                self.current = None;
            }
            if self.current.is_none() {
                self.current = shared.queue.pop();
            }
            let Some(block) = &self.current else {
                break;
            };
            if block.generation != generation {
                self.current = None;
                continue;
            }
            if at < block.start {
                break;
            }
            let index = (at - block.start) as usize;
            if index >= block.len {
                self.current = None;
                continue;
            }
            let gain = f32::from_bits(shared.gain.load(Ordering::Acquire));
            let frame = block.samples[index].map(|sample| (sample * gain).clamp(-1.0, 1.0));
            if self.output.generation() != generation {
                return [0.0; 2];
            }
            shared.position.store(at + 1, Ordering::Release);
            shared.waiting.store(false, Ordering::Release);
            self.starved = false;
            return frame;
        }
        shared.waiting.store(true, Ordering::Release);
        if !self.starved {
            shared.underruns.fetch_add(1, Ordering::Relaxed);
            self.starved = true;
        }
        [0.0; 2]
    }
}

impl<S: Source<Item = f32>> Iterator for Routed<S> {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let live = self.game.next().unwrap_or(0.0);
        if self.channel == 0 {
            self.frame = self.frame();
        }
        let value = if self.output.0.exclusive.load(Ordering::Acquire) {
            if self.output.0.playing.load(Ordering::Acquire)
                && self.generation == self.output.generation()
            {
                self.frame[self.channel]
            } else {
                0.0
            }
        } else {
            live
        };
        self.channel = (self.channel + 1) % PREVIEW_CHANNELS as usize;
        Some(value)
    }
}

impl<S: Source<Item = f32>> Source for Routed<S> {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        PREVIEW_CHANNELS
    }
    fn sample_rate(&self) -> u32 {
        self.output.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests;
