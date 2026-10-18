//! Stable logical tracks survive presentation lane repacking and share playback policy.

use crate::{Id, Project, StudioError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "kind", content = "index", rename_all = "snake_case")]
pub enum Track {
    Video(u16),
    Camera,
    Effects,
    Audio,
    Titles,
}

impl Track {
    /// Names logical owners once; physical overlap rows retain the same controls.
    pub fn label(self) -> String {
        match self {
            Self::Video(index) => format!("Video {}", index + 1),
            Self::Camera => "Camera".into(),
            Self::Effects => "Effects".into(),
            Self::Audio => "Audio".into(),
            Self::Titles => "Titles".into(),
        }
    }

    /// Rejects out-of-range video lanes before persisted track policy is consumed.
    pub fn valid(self) -> bool {
        !matches!(self, Self::Video(index) if index >= crate::model::MAX_VIDEO_TRACKS)
    }
}

/// Resolves a stable timeline identity to its owner without consulting painted lane positions.
pub fn item_track(project: &Project, id: Id) -> Result<Track, StudioError> {
    if let Some(clip) = project.clips.iter().find(|clip| clip.id == id) {
        return Ok(Track::Video(clip.track));
    }
    if project.shots.iter().any(|item| item.id == id) {
        return Ok(Track::Camera);
    }
    if project.effects.iter().any(|item| item.id == id) {
        return Ok(Track::Effects);
    }
    if project.audio.iter().any(|item| item.id == id) {
        return Ok(Track::Audio);
    }
    if project.titles.iter().any(|item| item.id == id) {
        return Ok(Track::Titles);
    }
    Err(StudioError::Missing {
        kind: "timeline track",
        id: id.0,
    })
}

impl Project {
    /// Clip mute, logical audio mute and linked visual-track mute agree in preview and export.
    pub fn audio_muted(&self, audio: &crate::model::AudioClip) -> bool {
        audio.muted
            || self.muted_tracks.contains(&Track::Audio)
            || audio
                .linked_clip
                .and_then(|id| self.clips.iter().find(|clip| clip.id == id))
                .is_some_and(|clip| {
                    !clip.enabled || self.muted_tracks.contains(&Track::Video(clip.track))
                })
    }
}
