//! Durable media manifest; the library never treats a POV proxy as a free-camera world replay.

use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};

use super::*;
use crate::{automation::Highlight, time::MAX_MEDIA_MICROS};

/// A versioned optional scene owner reports durable completion independently of the POV proxy.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureTrack {
    pub schema: String,
    pub directory: String,
    pub complete: bool,
    pub error: Option<String>,
}

impl CaptureTrack {
    /// Scene tracks resolve one sibling directory only; schemas never grant arbitrary path access.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema.is_empty()
            || self.schema.len() > 128
            || self.directory.is_empty()
            || self.directory.len() > 128
            || matches!(self.directory.as_str(), "." | "..")
            || !self
                .directory
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            || self.error.as_ref().is_some_and(|error| error.len() > 2048)
            || self.complete && self.error.is_some()
        {
            return Err("Invalid captured scene-track metadata.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureManifest {
    pub version: u32,
    pub settings: CaptureSettings,
    pub frames: u64,
    pub size: [u32; 2],
    pub duration: MediaTime,
    pub game_audio: bool,
    pub highlights: Vec<Highlight>,
    pub omitted_highlights: u64,
    pub missed_video_frames: u64,
    pub dropped_audio_samples: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene_tracks: Vec<CaptureTrack>,
}

impl CaptureManifest {
    /// Builds an ordinary editable project on the storage worker, with independent game audio.
    /// Observed highlights become reversible metadata effects rather than baked video pixels.
    pub fn project(
        &self,
        name: &str,
        directory: &Path,
    ) -> Result<(crate::edit::Editor, Option<String>), crate::StudioError> {
        self.validate()
            .map_err(|_| crate::StudioError::Invalid("invalid captured media"))?;
        use crate::{
            Id, PlaybackRate, Project, Span,
            edit::Editor,
            model::{AudioClip, AudioRole, Clip, Framing, MediaKind, MediaSource},
        };
        let mut project = Project::new(name);
        project.export.fps = self.settings.fps;
        project.export.size = self.size;
        project.export.codec = self.settings.codec;
        project.export.encoder = self.settings.encoder;
        project.export.quality = self.settings.quality;
        let source = Id(1);
        let range = Span::new(MediaTime::ZERO, self.duration)?;
        project.sources.push(MediaSource {
            id: source,
            name: name.into(),
            path: directory.join(VIDEO_FILE),
            kind: MediaKind::Video,
            duration: self.duration,
            size: self.size,
            sha256: None,
        });
        project.clips.push(Clip {
            id: Id(2),
            source,
            source_range: range,
            at: MediaTime::ZERO,
            rate: PlaybackRate::default(),
            track: 0,
            framing: Framing::default(),
            enabled: true,
        });
        if self.game_audio {
            project.sources.push(MediaSource {
                id: Id(3),
                name: "Game audio".into(),
                path: directory.join(AUDIO_FILE),
                kind: MediaKind::Audio,
                duration: self.duration,
                size: [0, 0],
                sha256: None,
            });
            project.audio.push(AudioClip {
                id: Id(4),
                source: Id(3),
                linked_clip: Some(Id(2)),
                source_range: range,
                at: MediaTime::ZERO,
                rate: PlaybackRate::default(),
                role: AudioRole::Game,
                gain: 1.0,
                fade_in: MediaTime::ZERO,
                fade_out: MediaTime::ZERO,
                muted: false,
            });
        }
        let mut editor = Editor::new(project)?;
        let proposal = crate::automation::suggest(
            editor.project(),
            source,
            &self.highlights,
            &Default::default(),
        )?;
        if proposal.highlights != 0 {
            editor.apply("Automatic recording highlights", &proposal.commands)?;
        }
        let omitted = self
            .omitted_highlights
            .saturating_add(proposal.omitted as u64);
        let mut notice = if omitted != 0 {
            Some(format!(
                "{} highlights added; {omitted} observations exceeded capture or edit limits. You can still edit the complete video.",
                proposal.highlights
            ))
        } else if proposal.correlated != 0 {
            Some(format!(
                "{} highlights added. {} use correlated combat evidence and remain labelled as estimates.",
                proposal.highlights, proposal.correlated
            ))
        } else {
            None
        };
        for track in &self.scene_tracks {
            if !track.complete {
                let message = format!(
                    "Scene track {} is incomplete: {} The POV video remains editable.",
                    track.schema,
                    track.error.as_deref().unwrap_or("storage did not finish")
                );
                if let Some(notice) = &mut notice {
                    notice.push('\n');
                    notice.push_str(&message);
                } else {
                    notice = Some(message);
                }
            }
        }
        Ok((editor, notice))
    }

    /// Opens only this schema's bounded metadata; arbitrary paths in a recording are never executed.
    pub fn read(directory: &Path) -> Result<Self, String> {
        let path = directory.join(MANIFEST_FILE);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
            return Err("Recording manifest is not a bounded regular file.".into());
        }
        let reader = BufReader::new(File::open(&path).map_err(|error| error.to_string())?);
        let manifest: Self = serde_json::from_reader(reader.take(MAX_MANIFEST_BYTES + 1))
            .map_err(|error| error.to_string())?;
        manifest.validate()?;
        let video = std::fs::symlink_metadata(directory.join(VIDEO_FILE))
            .map_err(|error| error.to_string())?;
        if !video.is_file() || video.len() == 0 {
            return Err("The captured video is missing or empty.".into());
        }
        if manifest.game_audio
            && !std::fs::symlink_metadata(directory.join(AUDIO_FILE))
                .is_ok_and(|value| value.is_file())
        {
            return Err("The recorded game-audio track is missing.".into());
        }
        Ok(manifest)
    }

    /// Checks recorded metadata independently of the capture process before project admission.
    pub fn validate(&self) -> Result<(), String> {
        self.settings.validate()?;
        if self.scene_tracks.len() > MAX_CAPTURE_TRACKS {
            return Err("Too many captured scene tracks.".into());
        }
        for (index, track) in self.scene_tracks.iter().enumerate() {
            track.validate()?;
            if self.scene_tracks[..index]
                .iter()
                .any(|old| old.schema == track.schema || old.directory == track.directory)
            {
                return Err("Duplicate captured scene track.".into());
            }
        }
        if self.version != MANIFEST_VERSION
            || self.frames == 0
            || self.duration == MediaTime::ZERO
            || self.duration.0 > MAX_MEDIA_MICROS
            || self.size.iter().any(|&side| side == 0 || side > 32_768)
            || self.highlights.len() > MAX_CAPTURE_HIGHLIGHTS
            || self
                .settings
                .fps
                .frame_time(self.frames)
                .map_err(|error| error.to_string())?
                != self.duration
            || self
                .highlights
                .iter()
                .any(|event| event.at > self.duration || event.label.len() > 256)
        {
            return Err("Unsupported or invalid captured-media metadata.".into());
        }
        Ok(())
    }

    /// Publishes a synced manifest last; incomplete encodes remain distinguishable from finished media.
    pub(super) fn write(&self, directory: &Path) -> Result<(), String> {
        self.validate()?;
        let staged = directory.join("capture.saving");
        let file = File::options()
            .write(true)
            .create_new(true)
            .open(&staged)
            .map_err(|error| error.to_string())?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, self).map_err(|error| error.to_string())?;
        writer
            .flush()
            .and_then(|()| writer.get_ref().sync_all())
            .map_err(|error| error.to_string())?;
        if writer
            .get_ref()
            .metadata()
            .map_err(|error| error.to_string())?
            .len()
            > MAX_MANIFEST_BYTES
        {
            return Err("Captured highlight metadata exceeds its storage limit.".into());
        }
        drop(writer);
        std::fs::rename(staged, directory.join(MANIFEST_FILE)).map_err(|error| error.to_string())
    }
}
