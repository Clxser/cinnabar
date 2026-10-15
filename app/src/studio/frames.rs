//! A single asynchronous GPU readback feeds nonblocking live-video admission.

use super::*;
use bevy::render::render_resource::TextureFormat;
use recording_studio::video::{Frame, FramePushError, PixelLayout, frame_byte_len};

/// One readback at a time keeps delivery ordered and bounds GPU readback memory independently of FPS.
pub(super) fn capture(world: &mut World, active: &mut Active, at: MediaTime) -> Result<(), String> {
    let index = active
        .session
        .settings()
        .fps
        .frame_at(at)
        .map_err(|error| error.to_string())?;
    let sink = active.session.sink();
    if active.last_frame.is_some_and(|last| index <= last)
        || !sink.has_capacity()
        || active
            .pending
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return Ok(());
    }
    let pending = Pending(Arc::clone(&active.pending));
    if let Some(pose) = observations::camera(world) {
        active.session.camera(at, pose)?;
    }
    let captured = Arc::clone(&active.captured);
    let failed = active.errors.clone();
    active.last_frame = Some(index);
    world
        .resource_mut::<crate::hud_tools::FrameCapture>()
        .request(move |image| {
            let result = frame(image, index).and_then(|frame| match sink.try_push(frame) {
                Ok(()) => {
                    captured.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                }
                Err(FramePushError::Full(_)) => Ok(()),
                Err(FramePushError::Closed(_)) => {
                    Err("The live video encoder stopped accepting frames.".into())
                }
                Err(FramePushError::Invalid(_)) => {
                    Err("The captured frame exceeds the encoder's memory limit.".into())
                }
            });
            if let Err(error) = result {
                let _ = failed.try_send(error);
            }
            drop(pending);
        });
    Ok(())
}

struct Pending(Arc<AtomicBool>);
impl Drop for Pending {
    /// Failed or abandoned screenshot callbacks release their slot as well as successful readbacks.
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Validates before copying the shared screenshot's CPU pixels; codecs never receive padded rows.
fn frame(image: &Image, index: u64) -> Result<Frame, String> {
    let size = [image.width(), image.height()];
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
        .as_ref()
        .filter(|data| data.len() == expected)
        .ok_or("The captured image has no tightly packed pixel data.")?;
    Ok(Frame {
        index,
        size,
        layout,
        data: data.clone(),
    })
}
