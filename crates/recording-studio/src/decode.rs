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
        })
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
            self.process = Some(Process::start(&self.path, self.size, self.fps, anchor)?);
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
    fn start(path: &Path, size: [u32; 2], fps: FrameRate, at: MediaTime) -> Result<Self, String> {
        let length = frame_byte_len(size)?;
        if !std::fs::metadata(path)
            .map_err(|error| error.to_string())?
            .is_file()
        {
            return Err("Preview media is not a regular file.".into());
        }
        let executable = crate::video::find_ffmpeg()?;
        let mut command = Command::new(executable);
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-threads",
                "2",
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
            .args(["-an", "-sn", "-dn", "-filter_threads", "1"])
            .arg("-vf")
            .arg(format!(
                "fps={}/{},scale={}:{}:flags=area",
                fps.numerator, fps.denominator, size[0], size[1]
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
        let (sender, frames) = bounded(1);
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
