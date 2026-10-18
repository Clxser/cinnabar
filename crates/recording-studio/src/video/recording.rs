//! Live and fixed-clock capture share worker-owned, cancellable encoder IO and finalization.

use super::*;
use crate::media_process::MediaChild;

/// Each actual pipe operation gets a deadline; waiting for the next capture frame does not.
struct GuardedInput<'a, W, F> {
    inner: W,
    cancellation: &'a Cancellation,
    deadline: F,
}

impl<W: Write, F: FnMut() -> G, G> GuardedInput<'_, W, F> {
    /// An owned-process supervisor can close a blocked pipe even while this worker cannot run.
    fn operation<T>(&mut self, operation: impl FnOnce(&mut W) -> io::Result<T>) -> io::Result<T> {
        self.cancellation.check().map_err(io::Error::other)?;
        let _deadline = (self.deadline)();
        let result = operation(&mut self.inner);
        self.cancellation.check().map_err(io::Error::other)?;
        result
    }
}

impl<W: Write, F: FnMut() -> G, G> Write for GuardedInput<'_, W, F> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.operation(|inner| inner.write(data))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.operation(Write::flush)
    }
}

/// The raw recording is never staged away: a failed encode leaves any completed fragments in place.
pub(super) fn encode(
    ffmpeg: &Path,
    settings: &VideoSettings,
    frames: &Receiver<QueuedFrame>,
    options: EncodeOptions,
    encoder: Encoder,
    expected: &AtomicU64,
    cancellation: &Cancellation,
) -> Result<Summary, String> {
    let mut frames = frames.iter().map(QueuedFrame::into_frame).peekable();
    let Some(first) = frames.peek() else {
        return Ok(Summary {
            path: settings.path.clone(),
            frames: 0,
            size: None,
        });
    };
    cancellation.check()?;
    let (size, layout) = (first.size, first.layout);
    frame_byte_len(size)?;
    let result = (|| {
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
            .stderr(Stdio::inherit());
        let mut process =
            MediaChild::start_output(command, cancellation.clone(), &settings.path, 0)?;
        let written = {
            let stdin = process
                .stdin
                .take()
                .ok_or("Recording encoder input is unavailable.")?;
            let mut input = GuardedInput {
                inner: stdin,
                cancellation,
                deadline: || process.guard_io(),
            };
            if options.speed == EncodingSpeed::Live {
                write_live(&mut input, frames, size, layout, expected, settings.fps)
            } else {
                write_ordered(&mut input, frames, size, layout)
            }
        };
        // Input is closed before flushing. Preserve watchdog failures alongside any pipe error.
        let finished = process.finish();
        finish_result(written, finished)
    })();
    let written = result.map_err(|error| {
        format!(
            "Video recording stopped: {error} Any captured video remains at {}.",
            settings.path.display()
        )
    })?;
    Ok(Summary {
        path: settings.path.clone(),
        frames: written,
        size: Some([size[0] & !1, size[1] & !1]),
    })
}

/// Frame validation and process failures are both actionable; neither may be masked by the other.
fn finish_result(written: io::Result<u64>, finished: Result<(), String>) -> Result<u64, String> {
    match (written, finished) {
        (Ok(count), Ok(())) => Ok(count),
        (Err(error), Ok(())) => Err(error.to_string()),
        (Ok(_), Err(error)) => Err(error),
        (Err(write), Err(process)) => Err(format!("{write}; {process}")),
    }
}

/// Audio is muxed to a separate file; cancellation or a failed process leaves the silent take intact.
pub(super) fn mux(video: &Path, audio: &Path, cancellation: &Cancellation) -> Result<(), String> {
    mux_inner(video, audio, cancellation).map_err(|error| {
        format!(
            "Audio muxing failed: {error} The silent video remains at {} and audio at {}.",
            video.display(),
            audio.display()
        )
    })
}

/// The source sizes reserve time for FFmpeg's silent faststart copy without a fixed take-length cap.
fn mux_inner(video: &Path, audio: &Path, cancellation: &Cancellation) -> Result<(), String> {
    cancellation.check()?;
    let ffmpeg = find_ffmpeg()?;
    let parent = video
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let stem = video_staging_path(video);
    let staged = tempfile::Builder::new()
        .prefix(stem.file_stem().unwrap_or_default())
        .suffix(".mp4")
        .tempfile_in(parent)
        .map_err(|error| error.to_string())?
        .into_temp_path();
    let relocation_bytes = [video, audio].into_iter().fold(0_u64, |bytes, path| {
        bytes.saturating_add(std::fs::metadata(path).map_or(0, |metadata| metadata.len()))
    });
    let mut command = Command::new(ffmpeg);
    command
        .args(mux_arguments(video, audio, &staged))
        .stdin(Stdio::null())
        .stderr(Stdio::inherit());
    MediaChild::start_output(command, cancellation.clone(), &staged, relocation_bytes)?.finish()?;
    if let Err(error) =
        crate::media_process::sync_output(&staged).and_then(|()| cancellation.check())
    {
        return Err(retain_completed_mux(staged, error));
    }
    publish_mux(staged, video)
}

/// A completed mux that cannot replace the silent video is retained at its own recovery path.
fn publish_mux(staged: tempfile::TempPath, video: &Path) -> Result<(), String> {
    staged.persist(video).map_err(|error| {
        retain_completed_mux(
            error.path,
            format!("Cannot publish muxed video: {}.", error.error),
        )
    })
}

/// Completed media survives flush, cancellation and publication failures, including keep errors.
fn retain_completed_mux(staged: tempfile::TempPath, mut error: String) -> String {
    let retained = match staged.keep() {
        Ok(path) => path,
        Err(mut keep) => {
            keep.path.disable_cleanup(true);
            error.push_str(&format!(
                " Cannot mark the mux as permanent: {}.",
                keep.error
            ));
            keep.path.to_path_buf()
        }
    };
    format!(
        "{error} The completed mux remains at {}.",
        retained.display()
    )
}

#[cfg(test)]
mod tests;
