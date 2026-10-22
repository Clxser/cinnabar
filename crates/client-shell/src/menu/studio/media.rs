//! One cancellable media worker owns the platform chooser, probing and content hashing.

use super::*;
use recording_studio::{export::Cancellation, media::ProbedMedia};

pub(super) struct MediaJob {
    document: u64,
    source: Option<MediaSource>,
    pub(super) cancellation: Cancellation,
    result: crossbeam_channel::Receiver<Result<Option<MediaResult>, String>>,
}
enum MediaResult {
    Imported(ProbedMedia),
    Relinked(MediaSource),
}

impl Drop for MediaJob {
    /// Closing/replacing an editor cancels decoding and hashing without joining the worker.
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

impl Studio {
    /// Selects the source referenced by a selected visual or audio clip, including missing files.
    fn selected_media(&self) -> Result<MediaSource, String> {
        let project = self.project()?;
        let selected = self
            .view
            .selected
            .ok_or("Select a media source, video clip or audio clip to relink.")?;
        let id = project
            .clips
            .iter()
            .find(|clip| clip.id == selected)
            .map(|clip| clip.source)
            .or_else(|| {
                project
                    .audio
                    .iter()
                    .find(|clip| clip.id == selected)
                    .map(|clip| clip.source)
            })
            .unwrap_or(selected);
        project
            .sources
            .iter()
            .find(|source| source.id == id)
            .cloned()
            .ok_or_else(|| "Select a media source, video clip or audio clip to relink.".into())
    }

    /// File selection runs on the worker; no main-thread probe, hash, metadata stat or directory read.
    pub(super) fn begin_media(&mut self, relink: bool) -> Result<(), String> {
        if self.media_job.is_some() {
            return Err("Finish or cancel the current media operation first.".into());
        }
        if self.editor.is_none() {
            self.install(
                Editor::new(Project::new("Untitled project")).map_err(|error| error.to_string())?,
                None,
            );
        }
        let source = relink.then(|| self.selected_media()).transpose()?;
        let worker_source = source.clone();
        let cancellation = Cancellation::default();
        let worker_cancel = cancellation.clone();
        let (completed, result) = crossbeam_channel::bounded(1);
        std::thread::Builder::new()
            .name("studio-media-import".into())
            .spawn(move || {
                let result = (|| {
                    let Some(path) = pick(relink)? else {
                        return Ok(None);
                    };
                    worker_cancel.check()?;
                    if let Some(source) = worker_source {
                        recording_studio::media::relink(&source, &path, &worker_cancel)
                            .map(|source| Some(MediaResult::Relinked(source)))
                    } else {
                        recording_studio::media::import(&path, &worker_cancel)
                            .map(|media| Some(MediaResult::Imported(media)))
                    }
                })();
                let _ = completed.send(result);
            })
            .map_err(|error| error.to_string())?;
        self.media_job = Some(MediaJob {
            document: self.document,
            source,
            cancellation,
            result,
        });
        let workflow = &mut Arc::make_mut(&mut self.view).workflow;
        workflow.media_busy = true;
        workflow.media_label = Some(
            if relink {
                "Choose the moved source file. Content identity will be checked."
            } else {
                "Choose media to import. Probing and content hashing run in the background."
            }
            .into(),
        );
        Ok(())
    }

    /// Cancellation remains pending while a native chooser is open; closing it releases the worker.
    pub(super) fn cancel_media(&mut self) {
        if let Some(job) = &self.media_job {
            job.cancellation.cancel();
        }
        Arc::make_mut(&mut self.view).workflow.media_label =
            Some("Cancelling media operation. Close the file chooser if it is still open.".into());
    }

    /// A stale completion cannot import into a replacement document or overwrite a later relink.
    pub(super) fn poll_media(&mut self) {
        let result = match self.media_job.as_ref().map(|job| job.result.try_recv()) {
            Some(Ok(result)) => result,
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                Err("The media import worker stopped unexpectedly.".into())
            }
            _ => return,
        };
        let job = self.media_job.take().expect("media job exists");
        let workflow = &mut Arc::make_mut(&mut self.view).workflow;
        workflow.media_busy = false;
        workflow.media_label = None;
        if job.cancellation.cancelled() || job.document != self.document {
            return;
        }
        let result = result.and_then(|result| {
            match result {
                Some(MediaResult::Imported(media)) => {
                    let at = self.project()?.duration();
                    let commands = recording_studio::media::import_commands(self.project()?, media, at)?;
                    self.edit("Import media", &commands)?;
                    Arc::make_mut(&mut self.view).show_library = false;
                }
                Some(MediaResult::Relinked(source)) => {
                    if !self.project()?.sources.iter().any(|current| Some(current) == job.source.as_ref()) { return Err("The source changed while its replacement was being checked. Relink it again.".into()); }
                    self.edit("Relink media", &[Command::ReplaceSource { source }])?;
                }
                None => {}
            }
            Ok(())
        });
        if let Err(error) = result {
            Arc::make_mut(&mut self.view).error = Some(error);
        }
        self.publish_editor();
    }
}

/// Platform dialogs use launcher-host's existing chooser integration and one central media filter.
fn pick(relink: bool) -> Result<Option<PathBuf>, String> {
    let title = if relink {
        "Relink studio media"
    } else {
        "Import studio media"
    };
    let patterns = recording_studio::media::IMPORT_EXTENSIONS
        .iter()
        .map(|extension| format!("*.{extension}"))
        .collect::<Vec<_>>();
    #[cfg(windows)]
    {
        launcher_host::desktop::windows::pick_file(
            title,
            &format!("Media files\0{}\0All files\0*.*\0\0", patterns.join(";")),
        )
        .map_err(|error| format!("The media chooser could not open ({error:#x})."))
    }
    #[cfg(not(windows))]
    {
        #[cfg(target_os = "linux")]
        if let Ok(path) = launcher_host::desktop::dbus::pick_file(title, "Media files", &patterns) {
            return Ok(path);
        }
        let mut command = if cfg!(target_os = "macos") {
            let mut command = std::process::Command::new("osascript");
            command.args([
                "-e",
                &format!("POSIX path of (choose file with prompt \"{title}\")"),
            ]);
            command
        } else {
            let mut command = std::process::Command::new("zenity");
            command.args([
                "--file-selection",
                &format!("--title={title}"),
                &format!("--file-filter=Media files | {}", patterns.join(" ")),
            ]);
            command
        };
        let output = command
            .output()
            .map_err(|error| format!("The media chooser is unavailable: {error}"))?;
        if !output.status.success() {
            return Ok(None);
        }
        let path = String::from_utf8(output.stdout)
            .map_err(|_| "The chosen media path is not valid UTF-8.")?;
        let path = path.trim_end_matches(['\r', '\n']);
        if path.len() > recording_studio::settings::MAX_PATH_BYTES {
            return Err("The selected media path is too long.".into());
        }
        Ok((!path.is_empty()).then(|| PathBuf::from(path)))
    }
}
