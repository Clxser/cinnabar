//! Interrupted native takes remain immutable; recovery admits their common committed scene prefix.

use std::path::{Path, PathBuf};

use recording_studio::{
    Id, MediaTime, PlaybackRate, Project, Span,
    archive::RecordingInfo,
    capture::{CameraRecording, CaptureSettings},
    edit::Editor,
    export::Cancellation,
    model::{Clip, Framing, MediaKind, MediaSource},
};

/// Restores verified native frames and independently recoverable companions into a separate unsaved project.
pub fn recover_project(
    parent: &Path,
    cancellation: &Cancellation,
) -> Result<recording_studio::capture::companions::RecoveredCapture, String> {
    let (editor, settings, parent) = verified_project(parent, cancellation)?;
    recording_studio::capture::companions::recover(
        editor.project().as_ref().clone(),
        &parent,
        settings.game_audio,
        cancellation,
    )
}

/// Finalizers verify scene coverage without publishing companion media or changing any original file.
pub fn verify_prefix(parent: &Path, cancellation: &Cancellation) -> Result<MediaTime, String> {
    verified_project(parent, cancellation).map(|(editor, _, _)| editor.duration())
}

/// A single scene admission owns source-grid verification for both validation and actual recovery.
fn verified_project(
    parent: &Path,
    cancellation: &Cancellation,
) -> Result<(Editor, CaptureSettings, PathBuf), String> {
    cancellation.check()?;
    let parent = std::fs::canonicalize(parent).map_err(|error| error.to_string())?;
    let camera = CameraRecording::open(&parent).map_err(|error| error.to_string())?;
    let settings = camera
        .capture_settings(|| cancellation.cancelled())
        .map_err(|error| error.to_string())?;
    let info = camera.info().clone();
    drop(camera);
    let mut playback =
        crate::playback::ScenePlayback::open(&parent, crate::scene::DEFAULT_RESIDENT_BYTES)
            .map_err(|error| format!("Native tracks cannot be recovered: {error}"))?;
    let duration = playback.duration();
    let editor = admitted_project(&parent, &info, settings, duration, |at| {
        cancellation.check()?;
        let result = if playback.position().is_none() {
            playback.seek(at, || cancellation.cancelled())
        } else {
            playback.advance_to(at, || cancellation.cancelled())
        };
        result.map(|_| ()).map_err(|error| {
            format!(
                "Native recovery could not verify the scene at {:.3}s: {error}",
                at.seconds()
            )
        })
    })?;
    Ok((editor, settings, parent))
}

/// Admission covers the recorded source grid, including transient artwork absent at either endpoint.
fn admitted_project(
    parent: &Path,
    info: &RecordingInfo,
    settings: CaptureSettings,
    duration: MediaTime,
    mut validate: impl FnMut(MediaTime) -> Result<(), String>,
) -> Result<Editor, String> {
    settings.validate()?;
    if duration == MediaTime::ZERO {
        return Err("The native take has no common committed playback interval.".into());
    }
    let frames = settings
        .fps
        .frames_covering(duration)
        .map_err(|error| error.to_string())?;
    for frame in 0..frames {
        validate(
            settings
                .fps
                .frame_time(frame)
                .map_err(|error| error.to_string())?,
        )?;
    }
    validate(duration)?;
    let mut project = Project::new(&info.name);
    project.export.fps = settings.fps;
    project.export.size = settings.size.fit(info.viewport)?.map(|side| side & !1);
    project.export.codec = settings.codec;
    project.export.encoder = settings.encoder;
    project.export.quality = settings.quality;
    project.export.include_audio = false;
    project.sources.push(MediaSource {
        id: Id(1),
        name: info.name.clone(),
        path: parent.to_path_buf(),
        kind: MediaKind::Replay,
        duration,
        size: info.viewport,
        frame_rate: Some(settings.fps),
        sha256: None,
    });
    project.clips.push(Clip {
        id: Id(2),
        source: Id(1),
        source_range: Span::new(MediaTime::ZERO, duration).map_err(|error| error.to_string())?,
        at: MediaTime::ZERO,
        rate: PlaybackRate::default(),
        speed_ramp: None,
        track: 0,
        framing: Framing::default(),
        enabled: true,
    });
    Editor::recovered(project).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
