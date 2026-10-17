//! Bounded pipelined readbacks retain capture order without waiting on the gameplay thread.

use std::{collections::BTreeMap, sync::atomic::AtomicUsize};

use super::*;
use bevy::render::render_resource::TextureFormat;
use recording_studio::video::{Frame, FramePushError, PixelLayout, frame_byte_len};

const MAX_READBACKS: usize = 3;
const MAX_READBACK_BYTES: usize = 96 * 1024 * 1024;

pub(super) struct Readbacks {
    pending: Arc<AtomicUsize>,
    completed: Receiver<Completion>,
    sender: crossbeam_channel::Sender<Completion>,
    ready: BTreeMap<u64, Completion>,
    next: u64,
    deliver: u64,
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
        Ok(Some(Slot {
            sender: self.sender.clone(),
            completion: Some(Completion {
                sequence,
                frame: None,
                _permit: Permit(Arc::clone(&self.pending)),
            }),
        }))
    }
}

struct Completion {
    sequence: u64,
    frame: Option<Result<Frame, String>>,
    _permit: Permit,
}

struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    /// Successful, skipped and abandoned readbacks release the same bounded admission slot.
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct Slot {
    sender: crossbeam_channel::Sender<Completion>,
    completion: Option<Completion>,
}
impl Slot {
    /// Ownership of the screenshot pixels transfers directly to the encoder queue.
    fn complete(mut self, frame: Result<Frame, String>) {
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
    for completion in active.video_readbacks.completed.try_iter() {
        active
            .video_readbacks
            .ready
            .insert(completion.sequence, completion);
    }
    let sink = active.session.sink();
    while let Some(completion) = active
        .video_readbacks
        .ready
        .remove(&active.video_readbacks.deliver)
    {
        active.video_readbacks.deliver += 1;
        if let Some(frame) = completion.frame {
            match sink.try_push(frame?) {
                Ok(()) => {
                    active.captured.fetch_add(1, Ordering::Relaxed);
                }
                Err(FramePushError::Full(_)) => {}
                Err(FramePushError::Closed(_)) => {
                    return Err("The live video encoder stopped accepting frames.".into());
                }
                Err(FramePushError::Invalid(_)) => {
                    return Err("The captured frame exceeds the encoder's memory limit.".into());
                }
            }
        }
    }
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
    if active.last_frame.is_some_and(|last| index <= last) || !active.session.sink().has_capacity()
    {
        return Ok(());
    }
    let Some(slot) = active
        .video_readbacks
        .reserve(active.session.info().viewport)?
    else {
        return Ok(());
    };
    if let Some(pose) = observations::camera(world) {
        active.session.camera(at, pose)?;
    }
    active.last_frame = Some(index);
    let expected_size = active.session.info().viewport;
    world
        .resource_mut::<crate::hud_tools::FrameCapture>()
        .request_owned(move |image| slot.complete(frame(image, index, expected_size)));
    Ok(())
}

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
