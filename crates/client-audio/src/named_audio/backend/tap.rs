//! Passive, bounded capture of the exact mixed samples submitted to hardware.
//!
//! The callback copies into a fixed ring. It never allocates, writes files or waits for recording.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

use crossbeam_queue::ArrayQueue;
use rodio::Source;

pub const TAP_BLOCK_SAMPLES: usize = 1024;
const TAP_BLOCKS: usize = 32;

/// Offsets count interleaved samples, so missing blocks can be replaced with aligned silence.
pub struct LiveAudioBlock {
    pub offset: u64,
    pub len: usize,
    pub samples: [f32; TAP_BLOCK_SAMPLES],
    generation: u64,
}

pub(super) struct SharedTap {
    queue: ArrayQueue<LiveAudioBlock>,
    owner: AtomicBool,
    next: AtomicU64,
    active: AtomicU64,
    drained: AtomicU64,
    dropped: AtomicU64,
    rate: u32,
}

impl SharedTap {
    /// Allocates all callback storage before the source is submitted to hardware.
    pub(super) fn new(rate: u32) -> Arc<Self> {
        Arc::new(Self {
            queue: ArrayQueue::new(TAP_BLOCKS),
            owner: AtomicBool::new(false),
            next: AtomicU64::new(1),
            active: AtomicU64::new(0),
            drained: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            rate,
        })
    }

    /// One recording owns the consumer until its disk worker has finished draining.
    pub(super) fn begin(self: &Arc<Self>) -> Option<LiveAudioTap> {
        if self
            .owner
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return None;
        }
        let generation = match self
            .next
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |next| {
                next.checked_add(1)
            }) {
            Ok(generation) => generation,
            Err(_) => {
                self.owner.store(false, Ordering::Release);
                return None;
            }
        };
        while self.queue.pop().is_some() {}
        self.dropped.store(0, Ordering::Release);
        self.active.store(generation, Ordering::Release);
        Some(LiveAudioTap {
            shared: Arc::clone(self),
            generation,
        })
    }
}

/// A single recording's passive read handle. Dropping it never stops audible voices.
pub struct LiveAudioTap {
    shared: Arc<SharedTap>,
    generation: u64,
}

impl LiveAudioTap {
    /// Hardware-native capture rate; the channel count is the shared stereo output contract.
    pub fn sample_rate(&self) -> u32 {
        self.shared.rate
    }

    /// Drains at most one fixed block and never waits for the hardware callback.
    pub fn pop(&self) -> Option<LiveAudioBlock> {
        for _ in 0..TAP_BLOCKS {
            let block = self.shared.queue.pop()?;
            if block.generation == self.generation {
                return Some(block);
            }
        }
        None
    }

    /// Stops collecting new samples while allowing the last partial block to be drained.
    pub fn stop(&self) {
        let _ = self.shared.active.compare_exchange(
            self.generation,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// A stopped callback has published its final partial block and acknowledged this generation.
    pub fn drained(&self) -> bool {
        self.shared.drained.load(Ordering::Acquire) == self.generation
            && self.shared.queue.is_empty()
    }

    /// Reports sample loss explicitly; a slow disk cannot interrupt audible playback.
    pub fn dropped_samples(&self) -> u64 {
        self.shared.dropped.load(Ordering::Acquire)
    }
}

impl Drop for LiveAudioTap {
    fn drop(&mut self) {
        self.stop();
        self.shared.owner.store(false, Ordering::Release);
    }
}

pub(super) struct Tapped<S> {
    inner: S,
    shared: Arc<SharedTap>,
    generation: u64,
    cursor: u64,
    len: usize,
    block: [f32; TAP_BLOCK_SAMPLES],
    channel: u16,
    channels: u16,
}

impl<S: Source<Item = f32>> Tapped<S> {
    /// Wraps the common mixed output, preserving one hardware sample clock for every voice.
    pub(super) fn new(inner: S, shared: Arc<SharedTap>) -> Self {
        let channels = inner.channels().max(1);
        Self {
            inner,
            shared,
            generation: 0,
            cursor: 0,
            len: 0,
            block: [0.0; TAP_BLOCK_SAMPLES],
            channel: 0,
            channels,
        }
    }

    /// A full ring loses capture data only; samples still reach the speakers without waiting.
    fn flush(&mut self) {
        if self.len == 0 {
            return;
        }
        let block = LiveAudioBlock {
            offset: self.cursor - self.len as u64,
            len: self.len,
            samples: self.block,
            generation: self.generation,
        };
        if self.shared.queue.push(block).is_err() {
            self.shared
                .dropped
                .fetch_add(self.len as u64, Ordering::Relaxed);
        }
        self.len = 0;
    }
}

impl<S: Source<Item = f32>> Iterator for Tapped<S> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.inner.next()?;
        let requested = self.shared.active.load(Ordering::Acquire);
        if requested != self.generation && self.channel == 0 {
            self.flush();
            if self.generation != 0 {
                self.shared
                    .drained
                    .store(self.generation, Ordering::Release);
            }
            self.generation = requested;
            self.cursor = 0;
        }
        if self.generation != 0 {
            self.block[self.len] = sample;
            self.len += 1;
            self.cursor += 1;
            if self.len == TAP_BLOCK_SAMPLES {
                self.flush();
            }
        }
        self.channel += 1;
        if self.channel == self.channels {
            self.channel = 0;
        }
        Some(sample)
    }
}

impl<S: Source<Item = f32>> Source for Tapped<S> {
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }
    fn channels(&self) -> u16 {
        self.inner.channels()
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}
