//! Video recording: raw frames piped into an `ffmpeg` child in frame order, with captured
//! audio muxed in afterwards.

use std::{
    collections::BTreeMap,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
};

use crate::FrameRate;
use crate::model::{Encoder, ExportCodec};
use crossbeam_channel::{Receiver, Sender, TrySendError};

mod export;
mod options;
pub use export::VideoWriter;
pub use options::{EncodeOptions, EncodingSpeed};

/// Bounds out-of-order delivery and queued frames. Live capture uses nonblocking admission.
const QUEUE_FRAMES: usize = 8;
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
const MAX_LIVE_QUEUED_BYTES: usize = 128 * 1024 * 1024;

/// Byte order of a captured 8-bit, 4-channel frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelLayout {
    Rgba,
    Bgra,
}

impl PixelLayout {
    fn ffmpeg_name(self) -> &'static str {
        match self {
            Self::Rgba => "rgba",
            Self::Bgra => "bgra",
        }
    }
}

/// One tightly packed frame; `index` (from zero) orders out-of-order GPU readbacks.
#[derive(Debug, Clone)]
pub struct Frame {
    pub index: u64,
    pub size: [u32; 2],
    pub layout: PixelLayout,
    pub data: Vec<u8>,
}

/// Bounds a tightly packed RGBA/BGRA allocation before a readback is copied or queued.
pub fn frame_byte_len(size: [u32; 2]) -> Result<usize, String> {
    if size.contains(&0) {
        return Err("A captured frame must have nonzero dimensions.".into());
    }
    (size[0] as usize)
        .checked_mul(size[1] as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|&bytes| bytes <= MAX_FRAME_BYTES)
        .ok_or_else(|| "The captured frame exceeds its byte limit.".into())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoSettings {
    pub path: PathBuf,
    pub fps: FrameRate,
    pub codec: ExportCodec,
}

/// Locates `ffmpeg` on PATH.
pub fn find_ffmpeg() -> Result<PathBuf, String> {
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            "FFmpeg was not found on PATH. Install FFmpeg and restart Cinnabar to record video."
                .to_owned()
        })
}

/// Encoder arguments for raw frames of `size` and `layout` on stdin at a constant rate.
pub fn ffmpeg_arguments(
    settings: &VideoSettings,
    size: [u32; 2],
    layout: PixelLayout,
    output: &Path,
) -> Vec<String> {
    options::arguments(
        settings,
        size,
        layout,
        output,
        EncodeOptions::default(),
        Encoder::Cpu,
    )
}

/// Arguments that copy `video` and encode `audio` to AAC into `output`.
pub fn mux_arguments(video: &Path, audio: &Path, output: &Path) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-y", "-i"]
        .map(str::to_owned)
        .into();
    args.push(video.display().to_string());
    args.push("-i".into());
    args.push(audio.display().to_string());
    args.extend(
        [
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-shortest",
            "-movflags",
            "+faststart",
        ]
        .map(str::to_owned),
    );
    args.push(output.display().to_string());
    args
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub path: PathBuf,
    pub frames: u64,
    pub size: Option<[u32; 2]>,
}

/// Cloneable handle the capture path pushes frames through. Every clone shares one slot,
/// so finishing the [`Recorder`] closes them all and the encoder never waits on a straggler.
#[derive(Clone)]
pub struct FrameSink {
    sender: Arc<Mutex<Option<Sender<QueuedFrame>>>>,
    queued_bytes: Arc<AtomicUsize>,
}

pub enum FramePushError {
    Full(Frame),
    Closed(Frame),
    Invalid(Frame),
}

struct QueuedFrame {
    frame: Frame,
    charge: Option<FrameCharge>,
}
struct FrameCharge {
    bytes: usize,
    queued: Arc<AtomicUsize>,
}

impl Drop for FrameCharge {
    /// Rejected sends, worker errors and normal consumption release the same reserved bytes.
    fn drop(&mut self) {
        self.queued.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

impl QueuedFrame {
    /// Only the frame currently being encoded escapes the queue's pinned-memory accounting.
    fn into_frame(self) -> Frame {
        let Self { frame, charge } = self;
        drop(charge);
        frame
    }
}

impl FrameSink {
    /// Live capture never waits for an encoder or another capture callback.
    pub fn try_push(&self, frame: Frame) -> Result<(), FramePushError> {
        let sender = match self.sender.try_lock() {
            Ok(slot) => slot.clone(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(FramePushError::Full(frame)),
            Err(std::sync::TryLockError::Poisoned(_)) => return Err(FramePushError::Closed(frame)),
        };
        let Some(sender) = sender else {
            return Err(FramePushError::Closed(frame));
        };
        let bytes = frame.data.len();
        if frame_byte_len(frame.size).ok() != Some(bytes) {
            return Err(FramePushError::Invalid(frame));
        }
        if self
            .queued_bytes
            .try_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                queued
                    .checked_add(bytes)
                    .filter(|&value| value <= MAX_LIVE_QUEUED_BYTES)
            })
            .is_err()
        {
            return Err(FramePushError::Full(frame));
        }
        let queued = QueuedFrame {
            frame,
            charge: Some(FrameCharge {
                bytes,
                queued: Arc::clone(&self.queued_bytes),
            }),
        };
        sender.try_send(queued).map_err(|error| match error {
            TrySendError::Full(queued) => FramePushError::Full(queued.into_frame()),
            TrySendError::Disconnected(queued) => FramePushError::Closed(queued.into_frame()),
        })
    }

    /// Allows a live capture adapter to avoid scheduling an unnecessary GPU readback.
    pub fn has_capacity(&self) -> bool {
        self.queued_bytes.load(Ordering::Acquire) < MAX_LIVE_QUEUED_BYTES
            && self
                .sender
                .try_lock()
                .ok()
                .and_then(|slot| slot.clone())
                .is_some_and(|sender| !sender.is_full())
    }
    /// Blocks while the encoder is behind, so a fixed-clock capture never drops frames.
    pub fn push(&self, frame: Frame) -> Result<(), String> {
        if frame_byte_len(frame.size)? != frame.data.len() {
            return Err("The captured frame has invalid pixel storage.".into());
        }
        let sender = self.sender.lock().ok().and_then(|slot| slot.clone());
        sender
            .ok_or_else(|| "the recording has finished".to_owned())?
            .send(QueuedFrame {
                frame,
                charge: None,
            })
            .map_err(|_| "the encoder exited early; see ffmpeg's error output".to_owned())
    }

    fn close(&self) {
        if let Ok(mut slot) = self.sender.lock() {
            slot.take();
        }
    }
}

/// A running encode; ffmpeg starts with the first frame, whose size and layout it fixes.
pub struct Recorder {
    sink: FrameSink,
    writer: Option<JoinHandle<Result<Summary, String>>>,
    settings: VideoSettings,
    expected_frames: Arc<AtomicU64>,
}

impl Recorder {
    pub fn start(settings: VideoSettings) -> Result<Self, String> {
        Self::start_with_options(settings, EncodeOptions::default())
    }

    /// Native live recording can select hardware and fast presets without changing developer captures.
    pub fn start_with_options(
        settings: VideoSettings,
        options: EncodeOptions,
    ) -> Result<Self, String> {
        settings.fps.validate().map_err(|error| error.to_string())?;
        options.validate()?;
        let ffmpeg = find_ffmpeg()?;
        if let Some(parent) = settings.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        Self::spawn(settings, move |settings, frames, expected| {
            let encoder = options.select_encoder(&ffmpeg, settings.codec)?;
            encode(&ffmpeg, settings, frames, options, encoder, expected)
        })
    }

    /// Resolves hardware capabilities before capture starts. Run only on the startup IO worker.
    pub fn start_prepared(settings: VideoSettings, options: EncodeOptions) -> Result<Self, String> {
        settings.fps.validate().map_err(|error| error.to_string())?;
        options.validate()?;
        let ffmpeg = find_ffmpeg()?;
        let encoder = options.select_encoder(&ffmpeg, settings.codec)?;
        if let Some(parent) = settings.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        Self::spawn(settings, move |settings, frames, expected| {
            encode(&ffmpeg, settings, frames, options, encoder, expected)
        })
    }

    /// Runs `encoder` on the writer thread over every pushed frame.
    fn spawn(
        settings: VideoSettings,
        encoder: impl FnOnce(
            &VideoSettings,
            &Receiver<QueuedFrame>,
            &AtomicU64,
        ) -> Result<Summary, String>
        + Send
        + 'static,
    ) -> Result<Self, String> {
        let (sender, receiver) = crossbeam_channel::bounded(QUEUE_FRAMES);
        let thread_settings = settings.clone();
        let expected_frames = Arc::new(AtomicU64::new(0));
        let expected = Arc::clone(&expected_frames);
        let writer = thread::Builder::new()
            .name("studio-video-encoder".into())
            .spawn(move || encoder(&thread_settings, &receiver, &expected))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            sink: FrameSink {
                sender: Arc::new(Mutex::new(Some(sender))),
                queued_bytes: Arc::default(),
            },
            writer: Some(writer),
            settings,
            expected_frames,
        })
    }

    pub fn settings(&self) -> &VideoSettings {
        &self.settings
    }

    pub fn sink(&self) -> FrameSink {
        self.sink.clone()
    }

    /// An active writer cannot finish normally until its producer is closed.
    pub fn worker_finished(&self) -> bool {
        self.writer.as_ref().is_some_and(JoinHandle::is_finished)
    }

    /// Live capture fixes its final duration before closing readbacks, preserving audio alignment.
    pub fn set_final_frame_count(&self, frames: u64) {
        self.expected_frames.store(frames, Ordering::Release);
    }

    /// Closes every [`FrameSink`] clone and waits for ffmpeg; then muxes `audio` (a WAV) in.
    pub fn finish(mut self, audio: Option<&Path>) -> Result<Summary, String> {
        self.sink.close();
        let summary = match self.writer.take() {
            Some(writer) => writer
                .join()
                .map_err(|_| "encoder thread panicked".to_owned())??,
            None => return Err("recording already finished".into()),
        };
        let Some(audio) = audio.filter(|_| summary.frames > 0) else {
            return Ok(summary);
        };
        let ffmpeg = find_ffmpeg()?;
        let parent = summary
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let stem = video_staging_path(&summary.path);
        let staged = tempfile::Builder::new()
            .prefix(stem.file_stem().unwrap_or_default())
            .suffix(".mp4")
            .tempfile_in(parent)
            .map_err(|error| error.to_string())?
            .into_temp_path();
        let mut command = Command::new(ffmpeg);
        command
            .args(mux_arguments(&summary.path, audio, &staged))
            .stdin(Stdio::null());
        crate::media_process::hide_console(&mut command);
        let status = command
            .status()
            .map_err(|error| format!("start ffmpeg to mux audio: {error}"))?;
        if !status.success() {
            return Err(format!(
                "muxing audio failed ({status}); the silent video is at {}",
                summary.path.display()
            ));
        }
        std::fs::File::open(&staged)
            .and_then(|file| file.sync_all())
            .map_err(|error| error.to_string())?;
        staged
            .persist(&summary.path)
            .map_err(|error| error.to_string())?;
        Ok(summary)
    }
}

impl Drop for Recorder {
    /// Closes admission immediately; the encoding worker drains asynchronously during normal teardown.
    fn drop(&mut self) {
        self.sink.close();
        self.writer.take();
    }
}

fn video_staging_path(path: &Path) -> PathBuf {
    let mut name = path.file_stem().unwrap_or_default().to_os_string();
    name.push(".video.mp4");
    path.with_file_name(name)
}

fn encode(
    ffmpeg: &Path,
    settings: &VideoSettings,
    frames: &Receiver<QueuedFrame>,
    options: EncodeOptions,
    encoder: Encoder,
    expected: &AtomicU64,
) -> Result<Summary, String> {
    let mut frames = frames.iter().map(QueuedFrame::into_frame).peekable();
    let Some(first) = frames.peek() else {
        return Ok(Summary {
            path: settings.path.clone(),
            frames: 0,
            size: None,
        });
    };
    let (size, layout) = (first.size, first.layout);
    frame_byte_len(size)?;
    let mut command = Command::new(ffmpeg);
    command
        .args(options::arguments(
            settings,
            size,
            layout,
            &settings.path,
            options,
            encoder,
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    crate::media_process::hide_console(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("start ffmpeg: {error}"))?;
    let stdin = child.stdin.take().ok_or("ffmpeg stdin unavailable")?;
    let written = finish_child(child, stdin, |stdin| {
        if options.speed == EncodingSpeed::Live {
            write_live(stdin, frames, size, layout, expected, settings.fps)
        } else {
            write_ordered(stdin, frames, size, layout)
        }
    })?;
    Ok(Summary {
        path: settings.path.clone(),
        frames: written,
        size: Some([size[0] & !1, size[1] & !1]),
    })
}

fn finish_child(
    mut child: Child,
    mut stdin: ChildStdin,
    write: impl FnOnce(&mut ChildStdin) -> io::Result<u64>,
) -> Result<u64, String> {
    let written = write(&mut stdin).map_err(|error| error.to_string());
    drop(stdin);
    let status = child.wait().map_err(|error| error.to_string())?;
    let written = written?;
    if !status.success() {
        return Err(format!("ffmpeg exited with {status}"));
    }
    Ok(written)
}

/// Writes exact captures in frame order with a bounded reorder window.
pub(crate) fn write_ordered(
    out: &mut impl Write,
    frames: impl Iterator<Item = Frame>,
    size: [u32; 2],
    layout: PixelLayout,
) -> io::Result<u64> {
    let frame_bytes = frame_byte_len(size).map_err(io::Error::other)?;
    let mut held = BTreeMap::new();
    let mut next = 0;
    let mut written = 0;
    for frame in frames {
        if frame.size != size || frame.layout != layout || frame.data.len() != frame_bytes {
            return Err(io::Error::other(format!(
                "frame {} is {:?} {:?}, expected {size:?} {layout:?}; the window changed size",
                frame.index, frame.size, frame.layout
            )));
        }
        if frame.index < next || held.insert(frame.index, frame.data).is_some() {
            return Err(io::Error::other(
                "GPU readback frame indices were duplicated",
            ));
        }
        if held.len() > QUEUE_FRAMES {
            return Err(io::Error::other(
                "GPU readbacks exceeded the bounded reorder window",
            ));
        }
        while let Some(data) = held.remove(&next) {
            out.write_all(&data)?;
            written += 1;
            next += 1;
        }
    }
    if !held.is_empty() {
        return Err(io::Error::other(
            "Exact recording ended with a missing GPU readback",
        ));
    }
    Ok(written)
}

/// One live readback is in flight at a time. Missing capture slots repeat the preceding frame;
/// they never speed up a recording or shift the audio timeline when rendering or encoding is busy.
fn write_live(
    out: &mut impl Write,
    frames: impl Iterator<Item = Frame>,
    size: [u32; 2],
    layout: PixelLayout,
    expected: &AtomicU64,
    fps: FrameRate,
) -> io::Result<u64> {
    let frame_bytes = frame_byte_len(size).map_err(io::Error::other)?;
    let mut previous: Option<Vec<u8>> = None;
    let mut next = 0;
    let maximum_gap = fps
        .frame_at(crate::MediaTime(5_000_000))
        .map_err(io::Error::other)?;
    for frame in frames {
        if frame.size != size || frame.layout != layout || frame.data.len() != frame_bytes {
            return Err(io::Error::other(
                "recording dimensions or pixel layout changed; stop and start a new recording",
            ));
        }
        if frame.index < next {
            return Err(io::Error::other("live readbacks arrived out of order"));
        }
        if frame.index - next > maximum_gap {
            return Err(io::Error::other(
                "recording paused for over five seconds waiting for video; the captured prefix is preserved",
            ));
        }
        let held = previous.as_deref().unwrap_or(&frame.data);
        while next < frame.index {
            out.write_all(held)?;
            next += 1;
        }
        out.write_all(&frame.data)?;
        next += 1;
        previous = Some(frame.data);
    }
    if let Some(previous) = previous {
        let end = expected.load(Ordering::Acquire);
        if end.saturating_sub(next) > maximum_gap {
            return Err(io::Error::other(
                "recording stopped after a prolonged video stall; the captured prefix is preserved",
            ));
        }
        while next < end {
            out.write_all(&previous)?;
            next += 1;
        }
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(index: u64, fill: u8) -> Frame {
        Frame {
            index,
            size: [1, 1],
            layout: PixelLayout::Bgra,
            data: vec![fill; 4],
        }
    }

    #[test]
    fn readbacks_are_written_in_frame_order() {
        let frames = [frame(2, 2), frame(0, 0), frame(1, 1), frame(3, 3)];
        let mut out = Vec::new();
        let written =
            write_ordered(&mut out, frames.into_iter(), [1, 1], PixelLayout::Bgra).unwrap();
        assert_eq!(written, 4);
        assert_eq!(out, [[0; 4], [1; 4], [2; 4], [3; 4]].concat());
    }

    #[test]
    fn dropping_a_recorder_with_live_sinks_finishes_the_encode() {
        let settings = VideoSettings {
            path: PathBuf::from("unused.mp4"),
            fps: FrameRate {
                numerator: 60,
                denominator: 1,
            },
            codec: ExportCodec::H264,
        };
        let recorder = Recorder::spawn(settings, |settings, frames, _| {
            Ok(Summary {
                path: settings.path.clone(),
                frames: frames.iter().count() as u64,
                size: None,
            })
        })
        .unwrap();
        let sink = recorder.sink();
        let (done, finished) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            drop(recorder);
            let _ = done.send(());
        });
        assert!(
            finished
                .recv_timeout(std::time::Duration::from_secs(5))
                .is_ok(),
            "dropping the recorder deadlocked on an outstanding sink"
        );
        assert!(sink.push(frame(0, 0)).is_err());
    }

    #[test]
    fn resized_frames_are_rejected() {
        let mut resized = frame(1, 1);
        resized.size = [2, 1];
        let mut out = Vec::new();
        let frames = [frame(0, 0), resized].into_iter();
        assert!(write_ordered(&mut out, frames, [1, 1], PixelLayout::Bgra).is_err());
    }

    #[test]
    fn arguments_select_the_codec_layout_and_rate() {
        let settings = VideoSettings {
            path: PathBuf::from("clip.mp4"),
            fps: FrameRate {
                numerator: 60,
                denominator: 1,
            },
            codec: ExportCodec::Hevc,
        };
        let args = ffmpeg_arguments(&settings, [1920, 1080], PixelLayout::Bgra, &settings.path);
        let joined = args.join(" ");
        assert!(joined.contains("-pix_fmt bgra -video_size 1920x1080 -framerate 60 -i -"));
        assert!(joined.contains("libx265"));
        assert_eq!(args.last().unwrap(), "clip.mp4");
        let mux = mux_arguments(Path::new("v.mp4"), Path::new("a.wav"), Path::new("o.mp4"));
        assert!(mux.join(" ").contains("-c:v copy -c:a aac"));
        assert_eq!(
            video_staging_path(Path::new("out/clip.mp4")),
            Path::new("out/clip.video.mp4")
        );
    }
}
