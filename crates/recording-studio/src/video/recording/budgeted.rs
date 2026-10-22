//! Binary encoder output is drained through take admission rather than polling a growing file.

use crate::{
    disk::{DiskBudget, Metered, StagedFile},
    export::Cancellation,
    media_process::MediaChild,
};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    thread::JoinHandle,
};

pub(super) const PIPE: &str = "pipe:1";

pub(super) struct OutputTask {
    worker: Option<JoinHandle<Result<(), String>>>,
    cancellation: Cancellation,
}
impl OutputTask {
    /// The encoder worker retains physical output ownership through every success/error return.
    pub(super) fn finish(mut self) -> Result<(), String> {
        self.worker
            .take()
            .expect("owned output worker")
            .join()
            .map_err(|_| "The recording output worker panicked.".to_owned())?
    }
}
impl Drop for OutputTask {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.cancellation.cancel();
            let _ = worker.join();
        }
    }
}

/// Seek-free MP4 output is required before FFmpeg is started with its binary stdout pipe.
pub(super) fn arguments(mut arguments: Vec<String>) -> Result<Vec<String>, String> {
    let flags = arguments
        .iter()
        .position(|arg| arg == "-movflags")
        .ok_or("Streaming output flags are unavailable.")?;
    *arguments
        .get_mut(flags + 1)
        .ok_or("Streaming output flags are incomplete.")? =
        super::super::options::STREAMING_MP4_FLAGS.into();
    if arguments.len() <= flags + 2 {
        return Err("Streaming output destination is unavailable.".into());
    }
    arguments.pop();
    arguments.extend(["-f".into(), "mp4".into(), PIPE.into()]);
    Ok(arguments)
}

/// A separate pipe worker prevents input/output backpressure from deadlocking the video encoder.
pub(super) fn drain(
    process: &mut MediaChild,
    path: PathBuf,
    disk: DiskBudget,
    cancellation: Cancellation,
) -> Result<OutputTask, String> {
    let mut output = process
        .stdout
        .take()
        .ok_or("Recording encoder output pipe is unavailable.")?;
    let activity = process.activity_counter();
    let worker_cancellation = cancellation.clone();
    let worker = std::thread::Builder::new()
        .name("studio-video-storage".into())
        .spawn(move || {
            let file = std::fs::File::options()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|error| error.to_string())?;
            let mut file = Metered::file(file, disk).map_err(|error| error.to_string())?;
            copy(&mut output, &mut file, &worker_cancellation, || {
                activity.fetch_add(1, std::sync::atomic::Ordering::Release);
            })?;
            file.flush()
                .and_then(|()| file.sync_all())
                .and_then(|()| {
                    crate::disk::sync_directory(path.parent().unwrap_or_else(|| Path::new(".")))
                })
                .map_err(|error| error.to_string())
        })
        .map_err(|error| error.to_string())?;
    Ok(OutputTask {
        worker: Some(worker),
        cancellation,
    })
}

/// Copying is bounded and cancellable between chunks; failed writes retain the original completed fragments.
fn copy(
    input: &mut impl Read,
    output: &mut impl Write,
    cancellation: &Cancellation,
    mut progress: impl FnMut(),
) -> Result<(), String> {
    let mut buffer = [0; 64 * 1024];
    loop {
        cancellation.check()?;
        let count = input.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            return cancellation.check();
        }
        cancellation.check()?;
        output
            .write_all(&buffer[..count])
            .map_err(|error| error.to_string())?;
        progress();
    }
}

/// Shared-budget finalization writes a fragmented, seek-free MP4 into accounted staging.
pub(super) fn mux(
    video: &Path,
    audio: &Path,
    cancellation: &Cancellation,
    disk: DiskBudget,
) -> Result<(), String> {
    let parent = video
        .parent()
        .ok_or("Recorded video has no parent directory.")?;
    let mut staged =
        StagedFile::new(parent, ".mux-", ".mp4", disk).map_err(|error| error.to_string())?;
    let args = arguments(super::mux_arguments(video, audio, Path::new(PIPE)))?;
    let mut command = std::process::Command::new(super::find_ffmpeg()?);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit());
    let mut process = MediaChild::start(command, cancellation.clone())?;
    let output = process
        .stdout
        .take()
        .ok_or("Mux output pipe is unavailable.")?;
    let mut output = super::GuardedInput {
        inner: output,
        cancellation,
        deadline: || process.guard_io(),
    };
    let activity = process.activity_counter();
    let result = copy(
        &mut output,
        &mut staged.writer().map_err(|error| error.to_string())?,
        cancellation,
        || {
            activity.fetch_add(1, std::sync::atomic::Ordering::Release);
        },
    );
    drop(output);
    if let Err(error) = result {
        drop(process);
        return Err(error);
    }
    process.finish()?;
    if let Err(error) = staged
        .sync_all()
        .map_err(|error| error.to_string())
        .and_then(|()| cancellation.check())
    {
        return Err(staged.retain(error));
    }
    staged.persist_replace(video).map_err(|error| {
        error
            .file
            .retain(format!("Cannot publish muxed video: {}.", error.error))
    })?;
    crate::disk::sync_directory(parent)
        .map_err(|error| format!("Cannot sync muxed video directory: {error}."))?;
    Ok(())
}

impl<R: Read, F: FnMut() -> G, G> Read for super::GuardedInput<'_, R, F> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.operation(|input| input.read(bytes))
    }
}

#[cfg(test)]
mod tests;
