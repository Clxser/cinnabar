//! Bounded pipelined readbacks retain capture order without waiting on the gameplay thread.

use std::{collections::BTreeMap, sync::atomic::AtomicUsize};

use super::*;
use bevy::render::render_resource::TextureFormat;
use recording_studio::capture::{MAX_READBACK_BYTES, MAX_READBACKS};
use recording_studio::video::{Frame, FramePushError, PixelLayout, frame_byte_len};

pub(super) struct Readbacks {
    pending: Arc<AtomicUsize>,
    completed: Receiver<Completion>,
    sender: crossbeam_channel::Sender<Completion>,
    ready: BTreeMap<u64, Completion>,
    next: u64,
    deliver: u64,
    bytes: Arc<AtomicUsize>,
    latency: Arc<AtomicU64>,
    pub(super) encoder_stalls: u64,
    pub(super) readback_stalls: u64,
    last_attempt: Option<u64>,
}

impl Default for Readbacks {
    /// Slots include both GPU work and completed frames awaiting an earlier callback.
    fn default() -> Self {
        let (sender, completed) = bounded(MAX_READBACKS);
        Self {
            pending: Arc::default(),
            completed,
            sender,
            ready: BTreeMap::new(),
            next: 0,
            deliver: 0,
            bytes: Arc::default(),
            latency: Arc::default(),
            encoder_stalls: 0,
            readback_stalls: 0,
            last_attempt: None,
        }
    }
}

impl Readbacks {
    /// Finalization drains outstanding callbacks before closing the encoder.
    pub(super) fn pending(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }

    /// The frame owner alone admits work; callbacks can only release charged slots.
    fn reserve(&mut self, size: [u32; 2]) -> Result<Option<Slot>, String> {
        let bytes = frame_byte_len(size)?;
        let limit = (MAX_READBACK_BYTES / bytes.max(1)).min(MAX_READBACKS);
        if self.pending() >= limit {
            return Ok(None);
        }
        let sequence = self.next;
        self.next = self
            .next
            .checked_add(1)
            .ok_or("Recording readback sequence was exhausted.")?;
        self.pending.fetch_add(1, Ordering::AcqRel);
        self.bytes.fetch_add(bytes, Ordering::AcqRel);
        Ok(Some(Slot {
            sender: self.sender.clone(),
            started: std::time::Instant::now(),
            latency: Arc::clone(&self.latency),
            completion: Some(Completion {
                sequence,
                frame: None,
                _permit: Permit {
                    pending: Arc::clone(&self.pending),
                    bytes: Arc::clone(&self.bytes),
                    size: bytes,
                },
            }),
        }))
    }

    /// A full encoder retains the oldest completed readback and its admission permit for retry.
    fn drain(
        &mut self,
        mut push: impl FnMut(Frame) -> Result<(), FramePushError>,
    ) -> Result<u64, String> {
        for completion in self.completed.try_iter() {
            self.ready.insert(completion.sequence, completion);
        }
        let mut captured = 0;
        while let Some(mut completion) = self.ready.remove(&self.deliver) {
            if let Some(frame) = completion.frame.take() {
                let frame = match frame {
                    Ok(frame) => frame,
                    Err(error) => {
                        self.deliver += 1;
                        return Err(error);
                    }
                };
                match push(frame) {
                    Ok(()) => captured += 1,
                    Err(FramePushError::Full(frame)) => {
                        self.encoder_stalls = self.encoder_stalls.saturating_add(1);
                        completion.frame = Some(Ok(frame));
                        self.ready.insert(self.deliver, completion);
                        break;
                    }
                    Err(FramePushError::Closed(_)) => {
                        self.deliver += 1;
                        return Err("The live video encoder stopped accepting frames.".into());
                    }
                    Err(FramePushError::Invalid(_)) => {
                        self.deliver += 1;
                        return Err("The captured frame exceeds the encoder's memory limit.".into());
                    }
                }
            } else {
                self.readback_stalls = self.readback_stalls.saturating_add(1);
            }
            self.deliver += 1;
        }
        Ok(captured)
    }
}

struct Completion {
    sequence: u64,
    frame: Option<Result<Frame, String>>,
    _permit: Permit,
}

struct Permit {
    pending: Arc<AtomicUsize>,
    bytes: Arc<AtomicUsize>,
    size: usize,
}
impl Drop for Permit {
    /// Successful, skipped and abandoned readbacks release the same bounded admission slot.
    fn drop(&mut self) {
        self.pending.fetch_sub(1, Ordering::AcqRel);
        self.bytes.fetch_sub(self.size, Ordering::AcqRel);
    }
}

struct Slot {
    sender: crossbeam_channel::Sender<Completion>,
    completion: Option<Completion>,
    started: std::time::Instant,
    latency: Arc<AtomicU64>,
}
impl Slot {
    /// Ownership of the screenshot pixels transfers directly to the encoder queue.
    fn complete(mut self, frame: Result<Frame, String>) {
        self.latency.store(
            self.started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
            Ordering::Release,
        );
        if let Some(completion) = &mut self.completion {
            completion.frame = Some(frame);
        }
    }
}
impl Drop for Slot {
    /// Dropping an uncalled observer still delivers a skipped slot, preserving later frame order.
    fn drop(&mut self) {
        if let Some(completion) = self.completion.take() {
            let _ = self.sender.try_send(completion);
        }
    }
}

/// Transfers ready callbacks in request order; out-of-order GPU completion cannot corrupt video time.
pub(super) fn drain(active: &mut Active) -> Result<(), String> {
    let sink = active.session.sink();
    active.video_readbacks.drain(|frame| {
        sink.try_push(frame).inspect(|()| {
            active.captured.fetch_add(1, Ordering::Relaxed);
        })
    })?;
    Ok(())
}

/// Multiple bounded slots hide mapping latency while retaining one capture request per video frame.
pub(super) fn capture(world: &mut World, active: &mut Active, at: MediaTime) -> Result<(), String> {
    let index = active
        .session
        .settings()
        .fps
        .frame_at(at)
        .map_err(|error| error.to_string())?;
    // Native camera coverage must not depend on encoder or GPU readback availability.
    if active.last_camera_frame.is_none_or(|last| index > last) {
        if let Some(pose) = observations::camera(world) {
            active.session.camera(at, pose)?;
            active.last_camera_frame = Some(index);
        }
    }
    if active.last_frame.is_some_and(|last| index <= last) {
        return Ok(());
    }
    let first_attempt = active.video_readbacks.last_attempt != Some(index);
    active.video_readbacks.last_attempt = Some(index);
    if !active
        .session
        .sink()
        .has_capacity_for(active.session.video_size())
    {
        if first_attempt {
            active.video_readbacks.encoder_stalls =
                active.video_readbacks.encoder_stalls.saturating_add(1);
        }
        return Ok(());
    }
    let expected_size = active.session.video_size();
    let source_size = active.session.info().viewport;
    let Some(slot) = active.video_readbacks.reserve(expected_size)? else {
        if first_attempt {
            active.video_readbacks.readback_stalls =
                active.video_readbacks.readback_stalls.saturating_add(1);
        }
        return Ok(());
    };
    active.last_frame = Some(index);
    let mut capture = world.resource_mut::<crate::hud_tools::FrameCapture>();
    let hud = active.session.settings().hud;
    capture.request_sized(expected_size, source_size, hud, move |image| {
        slot.complete(image.and_then(|image| frame(image, index, expected_size)));
    })?;
    Ok(())
}

/// Reads bounded queue counters without locking or copying queued payloads.
pub(super) fn health(
    active: &Active,
    actor_prepare_micros: u64,
    sample_micros: u64,
) -> recording_studio::capture::CaptureHealth {
    let readbacks = &active.video_readbacks;
    let (encoder_frames, encoder_bytes) = active.session.sink().queued();
    recording_studio::capture::CaptureHealth {
        actor_prepare_micros,
        sample_micros,
        readback_micros: readbacks.latency.load(Ordering::Acquire),
        pending_readbacks: readbacks.pending() as u32,
        readback_bytes: readbacks.bytes.load(Ordering::Acquire) as u64,
        encoder_frames: encoder_frames as u32,
        encoder_bytes: encoder_bytes as u64,
        encoder_stalls: readbacks.encoder_stalls,
        readback_stalls: readbacks.readback_stalls,
    }
}

#[cfg(test)]
mod tests;

/// Checks tightly packed SDR pixels before taking their allocation; no additional full-frame clone.
fn frame(mut image: Image, index: u64, expected_size: [u32; 2]) -> Result<Frame, String> {
    let size = [image.width(), image.height()];
    if size != expected_size {
        return Err("The recording window changed size; stop and start a new recording.".into());
    }
    let expected = frame_byte_len(size)?;
    if size.iter().any(|&side| side < 2) {
        return Err("The recording window is minimized or too small.".into());
    }
    let layout = match image.texture_descriptor.format {
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => PixelLayout::Rgba,
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb => PixelLayout::Bgra,
        _ => return Err("This window's pixel format cannot be captured as SDR video.".into()),
    };
    let data = image
        .data
        .take()
        .filter(|data| data.len() == expected)
        .ok_or("The captured image has no tightly packed pixel data.")?;
    Ok(Frame {
        index,
        size,
        layout,
        data,
    })
}
