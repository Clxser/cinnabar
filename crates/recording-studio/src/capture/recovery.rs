//! Interrupted POV recovery creates a separate verified video and never rewrites the original take.

use crate::{MediaTime, Project, edit::Editor, export::Cancellation, media_process::MediaChild};
use std::{
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

/// Remuxes the decodable video prefix into a new file; missing or corrupt fragments remain an error.
/// Scene tracks and unfinished audio are deliberately excluded from the recovered POV project.
pub fn recover_project(
    directory: &Path,
    name: &str,
    cancellation: &Cancellation,
) -> Result<Editor, String> {
    cancellation.check()?;
    let source = directory.join(super::VIDEO_FILE);
    let metadata = std::fs::metadata(&source)
        .map_err(|error| format!("No recoverable captured video: {error}"))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > crate::media::MAX_MEDIA_BYTES
    {
        return Err("The interrupted take has no bounded video file to recover.".into());
    }
    let staged = tempfile::Builder::new()
        .prefix(".recover-")
        .suffix(".mp4")
        .tempfile_in(directory)
        .map_err(|error| error.to_string())?
        .into_temp_path();
    let mut command = Command::new(crate::video::find_ffmpeg()?);
    command
        .args([
            "-v",
            "error",
            "-nostdin",
            "-y",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(&source)
        .args([
            "-map",
            "0:v:0",
            "-an",
            "-c:v",
            "copy",
            "-movflags",
            "+faststart",
            "-fs",
        ])
        .arg(crate::media::MAX_MEDIA_BYTES.to_string())
        .arg(&staged)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    MediaChild::start_timed(command, cancellation.clone(), Duration::from_secs(120))?.finish()?;
    let mut media = crate::media::probe(&staged, cancellation)?;
    crate::media::thumbnail(&staged, cancellation)?;
    crate::media_process::sync_output(&staged)?;
    cancellation.check()?;
    let mut staged = staged;
    let mut published = None;
    for index in 1..=1024 {
        let path = directory.join(format!("Recovered {index}.mp4"));
        match staged.persist_noclobber(&path) {
            Ok(()) => {
                published = Some(path);
                break;
            }
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                staged = error.path
            }
            Err(error) => return Err(error.error.to_string()),
        }
    }
    media.source.path = published.ok_or("Recovered filenames are exhausted in this take.")?;
    media.source.name = format!("{name} (recovered POV)")
        .chars()
        .take(256)
        .collect();
    let mut project = Project::new(name);
    if let Some(fps) = media.source.frame_rate {
        project.export.fps = fps;
    }
    project.export.size = media.source.size;
    let commands = crate::media::import_commands(&project, media, MediaTime::ZERO)?;
    let mut editor = Editor::new(project).map_err(|error| error.to_string())?;
    editor
        .apply("Recover interrupted video", &commands)
        .map_err(|error| error.to_string())?;
    Ok(editor)
}
