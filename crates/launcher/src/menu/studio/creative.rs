//! Assisted editing publishes suggestions and worker status without changing the saved project.

use recording_studio::{
    Id, MediaTime, Project,
    camera::CameraShot,
    model::{AudioClip, AudioRole},
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreativeAction {
    Section(super::AssistSection),
    DetectBeats,
    CancelAnalysis,
    StepTempo(i8),
    StepOffset(i16),
    BeatHere,
    ApplyBeatGrid,
    ClearBeatGrid,
    CycleLookStrength,
    ToggleLookRange,
    SuggestShots,
    CancelShotAnalysis,
    InspectShot(usize),
    ApplyShot(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreativeStatus {
    pub section: super::AssistSection,
    pub analyzing: bool,
    pub tempo_tenths: u16,
    pub offset_millis: i64,
    pub analyzed_clip: Option<Id>,
    pub message: Option<String>,
    pub look_strength: u16,
    pub look_selected_range: bool,
    pub shots: Arc<[CameraShot]>,
    pub shot_analyzing: bool,
    pub shot_message: Option<String>,
}

impl Default for CreativeStatus {
    fn default() -> Self {
        Self {
            section: Default::default(),
            analyzing: false,
            tempo_tenths: 120 * recording_studio::audio::beats::TEMPO_SCALE,
            offset_millis: 0,
            analyzed_clip: None,
            message: None,
            look_strength: 100,
            look_selected_range: true,
            shots: Arc::default(),
            shot_analyzing: false,
            shot_message: None,
        }
    }
}

/// An explicit selection wins; otherwise use music under the playhead before other retained audio.
pub fn audio_clip(project: &Project, selected: Option<Id>, at: MediaTime) -> Option<&AudioClip> {
    project
        .audio
        .iter()
        .find(|clip| Some(clip.id) == selected)
        .or_else(|| {
            project.audio.iter().find(|clip| {
                Some(clip.source) == selected
                    || clip.linked_clip.is_some() && clip.linked_clip == selected
            })
        })
        .or_else(|| {
            project.audio.iter().find(|clip| {
                clip.role == AudioRole::Music && clip.span().is_ok_and(|span| span.contains(at))
            })
        })
        .or_else(|| {
            project
                .audio
                .iter()
                .find(|clip| clip.role == AudioRole::Music)
        })
        .or_else(|| {
            project
                .audio
                .iter()
                .find(|clip| clip.span().is_ok_and(|span| span.contains(at)))
        })
        .or_else(|| project.audio.first())
}
