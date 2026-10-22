//! Completed and recovered takes share initial game-audio project admission.

use crate::{
    Id, MediaTime, Project, Span, StudioError,
    model::{AudioClip, AudioRole, MediaKind, MediaSource},
};
use std::path::PathBuf;

/// Attaches only the available source interval; shorter audio remains unlinked rather than extending its duration.
pub(super) fn game_audio(
    project: &mut Project,
    visual: Id,
    path: PathBuf,
    duration: MediaTime,
) -> Result<Id, StudioError> {
    let clip = project
        .clips
        .iter()
        .find(|clip| clip.id == visual)
        .ok_or(StudioError::Missing {
            kind: "visual clip",
            id: visual.0,
        })?;
    let source_range = Span::new(clip.source_range.start, clip.source_range.end.min(duration))?;
    let source = project.next_id()?;
    let id = Id(source
        .0
        .checked_add(1)
        .ok_or(StudioError::IdentifiersExhausted)?);
    let audio = AudioClip {
        id,
        source,
        linked_clip: (source_range == clip.source_range).then_some(visual),
        source_range,
        at: clip.at,
        rate: clip.rate,
        speed_ramp: clip.speed_ramp.clone(),
        role: AudioRole::Game,
        gain: 1.0,
        fade_in: MediaTime::ZERO,
        fade_out: MediaTime::ZERO,
        muted: false,
    };
    project.sources.push(MediaSource {
        id: source,
        name: "Game audio".into(),
        path,
        kind: MediaKind::Audio,
        duration,
        size: [0, 0],
        frame_rate: None,
        sha256: None,
    });
    project.audio.push(audio);
    project.export.include_audio = true;
    Ok(source)
}
