//! Reversible scene selection does not read media or mutate the captured POV file.

use super::*;

impl Studio {
    /// Replaces one selected POV clip with a native-source reference; the worker validates scene coverage.
    pub(super) fn use_native_replay(&mut self) -> Result<(), String> {
        let project = self.project()?;
        let selected = self
            .view
            .selected
            .ok_or("Select a captured POV clip first.")?;
        let clip = project
            .clips
            .iter()
            .find(|clip| clip.id == selected)
            .ok_or("Select a captured POV clip first.")?;
        let video = project
            .sources
            .iter()
            .find(|source| source.id == clip.source)
            .ok_or("The selected clip's source is missing.")?;
        let path = recording_studio::capture::native_replay_directory(video)
            .ok_or("Only a captured POV clip can switch to its native replay.")?
            .to_path_buf();
        let id = project.next_id().map_err(|error| error.to_string())?;
        let source = MediaSource {
            id,
            path,
            name: format!("{} (3D replay)", video.name),
            kind: MediaKind::Replay,
            duration: video.duration,
            size: video.size,
            frame_rate: None,
            sha256: None,
        };
        let mut clip = clip.clone();
        clip.source = id;
        self.transport.pause();
        self.edit(
            "Use native replay",
            &[Command::AddSource { source }, Command::ReplaceClip { clip }],
        )?;
        Arc::make_mut(&mut self.view).notice = Some("Native replay uses captured scene tracks. Older or incomplete takes may only support POV video; Undo returns to that video.".into());
        Ok(())
    }
}
