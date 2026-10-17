//! Bounded local-video decoding. A preview/export worker owns every process and blocking read.

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Arc,
    thread::JoinHandle,
    time::Duration,
};

use crossbeam_channel::{Receiver, RecvTimeoutError, bounded};

use crate::{FrameRate, MediaTime, time::MICROS_PER_SECOND, video::frame_byte_len};
const RECEIVE_INTERVAL: Duration = Duration::from_millis(10);
const READ_AHEAD_BYTES: usize = 16 * 1024 * 1024;
const READ_AHEAD_FRAMES: usize = 4;

pub struct DecodedFrame {
    pub size: [u32; 2],
    pub rgba8: Arc<[u8]>,
}

pub struct VideoDecoder {
    path: PathBuf,
    size: [u32; 2],
    fps: FrameRate,
    process: Option<Process>,
    next: u64,
    last: Option<(u64, Arc<[u8]>)>,
    interactive: bool,
    hardware: bool,
}

struct Process {
    child: Child,
    frames: Option<Receiver<Result<Arc<[u8]>, String>>>,
    reader: Option<JoinHandle<()>>,
}

impl VideoDecoder {
    /// Metadata edits can retain an existing decoder when its actual media, extent and cadence are unchanged.
    pub fn matches(&self, path: &Path, size: [u32; 2], fps: FrameRate) -> bool {
        self.path == path && self.size == size && self.fps == fps
    }

    /// Stores only admitted configuration. Process startup remains deferred until an actual frame is needed.
    pub fn new(path: &Path, size: [u32; 2], fps: FrameRate) -> Result<Self, String> {
        fps.validate().map_err(|error| error.to_string())?;
        frame_byte_len(size)?;
        Ok(Self {
            path: path.to_path_buf(),
            size,
            fps,
            process: None,
            next: 0,
            last: None,
            interactive: false,
            hardware: false,
        })
    }

    /// Interactive previews use hardware decoding when available and a bounded bilinear preview scaler.
    /// Export retains the software decoder and area scaler selected by `new`.
    pub fn interactive(path: &Path, size: [u32; 2], fps: FrameRate) -> Result<Self, String> {
        let mut decoder = Self::new(path, size, fps)?;
        decoder.interactive = true;
        decoder.hardware = cfg!(windows);
        Ok(decoder)
    }

    /// A transport seek invalidates read-ahead even when the requested source time is nearby.
    pub fn seek(&mut self) {
        self.process.take();
        self.last = None;
    }

    /// Reports the decoder actually selected after any hardware fallback, without probing another process.
    pub const fn hardware(&self) -> bool {
        self.hardware
    }

    /// Adjacent forward requests reuse the decoder. Backward or distant seeks restart accurate local-file decoding.
    /// The caller must run this on an IO worker; cancellation kills only this decoder's owned child process.
    pub fn frame(
        &mut self,
        at: MediaTime,
        cancelled: impl Fn() -> bool,
    ) -> Result<DecodedFrame, String> {
        let target = self.fps.frame_at(at).map_err(|error| error.to_string())?;
        if cancelled() {
            self.process.take();
            return Err("Preview decode cancelled.".into());
        }
        if let Some((index, rgba8)) = &self.last
            && *index == target
        {
            return Ok(DecodedFrame {
                size: self.size,
                rgba8: Arc::clone(rgba8),
            });
        }
        let result = self.decode(target, &cancelled);
        if result.is_err() && self.hardware && !cancelled() {
            // A missing device, unsupported codec or failed hardware process retries this exact frame once.
            self.hardware = false;
            self.seek();
            return self.decode(target, &cancelled);
        }
        result
    }

    /// Nearby forward frames reuse read-ahead; distant timeline cuts seek directly to their source frame.
    fn decode(
        &mut self,
        target: u64,
        cancelled: &impl Fn() -> bool,
    ) -> Result<DecodedFrame, String> {
        let maximum_gap = self
            .fps
            .frames_covering(MediaTime(MICROS_PER_SECOND))
            .map_err(|error| error.to_string())?;
        if self.process.is_none()
            || target < self.next
            || target.saturating_sub(self.next) > maximum_gap
        {
            self.process.take();
            let anchor = self
                .fps
                .frame_time(target)
                .map_err(|error| error.to_string())?;
            self.process = Some(Process::start(
                &self.path,
                self.size,
                self.fps,
                anchor,
                self.interactive,
                self.hardware,
            )?);
            self.next = target;
        }
        while self.next <= target {
            if cancelled() {
                self.process.take();
                return Err("Preview decode cancelled.".into());
            }
            let result = self
                .process
                .as_ref()
                .and_then(|process| process.frames.as_ref())
                .ok_or("Preview decoder stopped.")?
                .recv_timeout(RECEIVE_INTERVAL);
            match result {
                Ok(Ok(bytes)) => {
                    self.last = Some((self.next, bytes));
                    self.next += 1;
                }
                Ok(Err(error)) => {
                    self.process.take();
                    return Err(error);
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    self.process.take();
                    return Err("The video ended before the requested frame.".into());
                }
            }
        }
        let rgba8 = Arc::clone(&self.last.as_ref().ok_or("No video frame was decoded.")?.1);
        Ok(DecodedFrame {
            size: self.size,
            rgba8,
        })
    }
}

impl Process {
    /// Restricts demuxer protocols to local files/pipes and bounds the raw-frame handoff independently of FFmpeg.
    fn start(
        path: &Path,
        size: [u32; 2],
        fps: FrameRate,
        at: MediaTime,
        interactive: bool,
        hardware: bool,
    ) -> Result<Self, String> {
        let length = frame_byte_len(size)?;
        if !std::fs::metadata(path)
            .map_err(|error| error.to_string())?
            .is_file()
        {
            return Err("Preview media is not a regular file.".into());
        }
        let executable = crate::video::find_ffmpeg()?;
        let mut command = Command::new(executable);
        if hardware {
            command.args(["-hwaccel", "d3d11va"]);
        }
        let threads = if interactive && !hardware { "4" } else { "2" };
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-threads",
                threads,
                "-protocol_whitelist",
                "file,pipe",
            ])
            .arg("-ss")
            .arg(format!(
                "{}.{:06}",
                at.0 / MICROS_PER_SECOND,
                at.0 % MICROS_PER_SECOND
            ))
            .arg("-i")
            .arg(path)
            .args([
                "-an",
                "-sn",
                "-dn",
                "-filter_threads",
                if interactive { "2" } else { "1" },
            ])
            .arg("-vf")
            .arg(format!(
                "fps={}/{},scale={}:{}:flags={}",
                fps.numerator,
                fps.denominator,
                size[0],
                size[1],
                if interactive { "bilinear" } else { "area" }
            ))
            .args([
                "-threads", "1", "-pix_fmt", "rgba", "-f", "rawvideo", "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        crate::media_process::hide_console(&mut command);
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        let Some(mut stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Video decoder stdout is unavailable.".into());
        };
        let capacity = if interactive {
            (READ_AHEAD_BYTES / length).clamp(1, READ_AHEAD_FRAMES)
        } else {
            1
        };
        let (sender, frames) = bounded(capacity);
        let reader = match std::thread::Builder::new()
            .name("studio-video-frames".into())
            .spawn(move || {
                loop {
                    let mut bytes = vec![0_u8; length];
                    let result = stdout
                        .read_exact(&mut bytes)
                        .map(|()| Arc::<[u8]>::from(bytes))
                        .map_err(|error| format!("Video frame decoding stopped: {error}"));
                    let failed = result.is_err();
                    if sender.send(result).is_err() || failed {
                        break;
                    }
                }
            }) {
            Ok(reader) => reader,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        };
        Ok(Self {
            child,
            frames: Some(frames),
            reader: Some(reader),
        })
    }
}

impl Drop for Process {
    /// Dropping the receiver releases a blocked send; killing the owned child releases a blocked pipe read.
    fn drop(&mut self) {
        self.frames.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
