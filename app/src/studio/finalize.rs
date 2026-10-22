//! Blocking track drains and prefix validation stay on the capture's existing finalization worker.

use recording_studio::{MediaTime, capture::CaptureSummary, export::Cancellation};

use super::{Active, NativeCapture, bounded, failures};

pub(super) type FinishResult = Result<CaptureSummary, Failure>;

#[derive(Debug)]
pub(super) struct Failure {
    pub message: String,
    pub verified_prefix: Option<MediaTime>,
}

impl From<String> for Failure {
    /// Ordinary IO failures retain their explanation without claiming a verified scene interval.
    fn from(message: String) -> Self {
        Self {
            message,
            verified_prefix: None,
        }
    }
}

/// Closes each owned producer off-thread, retaining failed native journals for explicit recovery.
pub(super) fn start(state: &mut NativeCapture, mut active: Active, duration: MediaTime) {
    let (completed, finishing) = bounded(1);
    let result = std::thread::Builder::new()
        .name("studio-capture-finish".into())
        .spawn(move || {
            let directory = active.session.directory().to_path_buf();
            let mode = active.session.settings().mode;
            let metadata_error = failures::finish(&mut active, duration);
            let audio_result = active
                .audio
                .take()
                .map(super::audio::AudioWorker::finish)
                .transpose();
            let (audio, audio_error) = match audio_result {
                Ok(audio) => (audio, None),
                Err(error) => (None, Some(error)),
            };
            let expected = active
                .session
                .settings()
                .fps
                .frames_covering(duration)
                .unwrap_or(0);
            let missed =
                expected.saturating_sub(active.captured.load(std::sync::atomic::Ordering::Acquire));
            let interruption = active
                .failure
                .or(audio_error)
                .or(metadata_error)
                .or_else(|| {
                    if mode.records_video() {
                        None
                    } else {
                        active.native_failures.message().map(str::to_owned)
                    }
                });
            let summary = active.session.finish(
                duration,
                audio.as_ref().map(|(path, _)| path.as_path()),
                missed,
                audio.as_ref().map_or(0, |(_, dropped)| *dropped),
            );
            let result = match (summary, interruption) {
                (Ok(summary), None) => Ok(summary),
                (result, interruption) if !mode.records_video() => {
                    let error = interruption
                        .or_else(|| result.err())
                        .unwrap_or_else(|| "Native recording was interrupted.".into());
                    Err(native_failure_report(&error, || {
                        recording_scene::recovery::verify_prefix(
                            &directory,
                            &Cancellation::default(),
                        )
                    }))
                }
                (Ok(_), Some(error)) => {
                    Err(format!("Recording saved with an interruption: {error}").into())
                }
                (Err(error), Some(capture_error)) => Err(format!(
                    "Recording stopped: {capture_error}. Saving also failed: {error}"
                )
                .into()),
                (Err(error), None) => Err(error.into()),
            };
            let _ = completed.send(result);
        });
    match result {
        Ok(_) => state.finishing = Some(finishing),
        Err(error) => super::fail(state, error.to_string()),
    }
}

/// Only a fully verified common prefix is described as recoverable; a failed validation never claims saved coverage.
fn native_failure_report(
    error: &str,
    verify: impl FnOnce() -> Result<MediaTime, String>,
) -> Failure {
    match verify() {
        Ok(duration) if duration > MediaTime::ZERO => Failure {
            message: format!(
                "3D replay stopped: {error}. Verified common committed prefix: {:.3}s. Recover this interrupted take in Studio; the original journals are retained.",
                duration.seconds()
            ),
            verified_prefix: Some(duration),
        },
        Ok(_) => format!(
            "3D replay stopped: {error}. No common committed playback interval was available. The original journals are retained."
        ).into(),
        Err(recovery) => format!(
            "3D replay stopped: {error}. The common committed prefix could not be verified: {recovery}. The original journals are retained for recovery."
        ).into(),
    }
}

#[cfg(test)]
mod tests;
