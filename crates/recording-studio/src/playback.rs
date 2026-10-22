//! Allocation-reusing preview plans and a caller-driven transport shared by playback and export.

use std::{collections::BTreeMap, sync::Arc};

use crate::{
    FrameRate, Id, MediaTime, PlaybackRate, Project, Span, StudioError,
    camera::{ActorPose, CameraPose, CameraTargetRef, CompiledCameraPath},
    effect::EffectSample,
    model::{Framing, TransitionKind},
    retime::TimeMap,
};

mod camera_blend;
#[cfg(test)]
mod hud_tests;
mod index;
pub mod monitoring;
use index::{IntervalIndex, Slot};

/// Each admitted visual track can expose both members of its active dissolve.
pub const MAX_VISUAL_LAYERS: usize = crate::model::MAX_VIDEO_TRACKS as usize * 2;

/// The timeline clock is deliberately independent of the application's live world clock.
pub struct Transport {
    position: MediaTime,
    playing: bool,
    rate: PlaybackRate,
    remainder: u128,
    looping: Option<Span>,
    epoch: u64,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            position: MediaTime::ZERO,
            playing: false,
            rate: PlaybackRate::default(),
            remainder: 0,
            looping: None,
            epoch: 0,
        }
    }
}

impl Transport {
    pub const fn position(&self) -> MediaTime {
        self.position
    }
    pub const fn playing(&self) -> bool {
        self.playing
    }
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }
    pub const fn rate(&self) -> PlaybackRate {
        self.rate
    }

    /// Monitoring loops are runtime state and never become export or source trims.
    pub const fn looping(&self) -> Option<Span> {
        self.looping
    }

    /// A seek invalidates temporal renderer, interpolation and audio histories through its epoch.
    pub fn seek(&mut self, position: MediaTime, duration: MediaTime) {
        self.position = MediaTime(position.0.min(duration.0));
        self.remainder = 0;
        self.epoch = self.epoch.wrapping_add(1);
    }

    /// Starting at the end restarts the timeline rather than appearing to ignore Play.
    pub fn play(&mut self, duration: MediaTime) {
        if self.position >= duration {
            self.seek(MediaTime::ZERO, duration);
        }
        self.playing = duration != MediaTime::ZERO;
    }

    /// Pausing keeps the current pose, effects and edit selection unchanged.
    pub fn pause(&mut self) {
        self.playing = false;
    }

    /// Playback speed changes only the preview clock; clip retiming is a separate saved edit.
    pub fn set_rate(&mut self, rate: PlaybackRate) -> Result<(), StudioError> {
        rate.validate()?;
        if self.rate != rate {
            self.rate = rate;
            self.remainder = 0;
            self.epoch = self.epoch.wrapping_add(1);
        }
        Ok(())
    }

    /// Loop bounds belong to the transport and do not trim or overwrite source media.
    pub fn set_loop(&mut self, span: Option<Span>, duration: MediaTime) -> Result<(), StudioError> {
        if let Some(span) = span {
            span.validate()?;
            if span.end > duration {
                return Err(StudioError::Invalid("loop extends beyond the project"));
            }
        }
        self.looping = span;
        Ok(())
    }

    /// Caller-supplied elapsed time advances with a retained rational remainder, avoiding drift.
    pub fn advance(&mut self, elapsed: MediaTime, duration: MediaTime) {
        self.advance_until(elapsed, duration, MediaTime(u64::MAX));
    }

    /// A device-consumed sample cursor advances monotonically under the same displayed-frame backpressure.
    pub fn synchronize_audio(
        &mut self,
        position: MediaTime,
        duration: MediaTime,
        available: MediaTime,
    ) {
        if !self.playing {
            return;
        }
        self.align_audio(position, duration, available);
        self.complete_advance(u128::from(self.position.0), duration);
    }

    /// A confirmed device pause updates the frozen picture without seeking or restarting audio history.
    pub fn reconcile_paused_audio(
        &mut self,
        position: MediaTime,
        duration: MediaTime,
        available: MediaTime,
    ) {
        if !self.playing {
            self.align_audio(position, duration, available);
        }
    }

    /// Both audible and paused cursor admissions obey the same monotone displayed-frame boundary.
    fn align_audio(&mut self, position: MediaTime, duration: MediaTime, available: MediaTime) {
        self.position = position
            .max(self.position)
            .min(duration)
            .min(available.max(self.position));
        self.remainder = 0;
    }

    /// Backpressure limits unseen preview time without seeking, rewinding or retaining catch-up debt.
    pub fn advance_until(&mut self, elapsed: MediaTime, duration: MediaTime, available: MediaTime) {
        if !self.playing {
            return;
        }
        let product = u128::from(elapsed.0) * u128::from(self.rate.numerator) + self.remainder;
        let denominator = u128::from(self.rate.denominator);
        let delta = product / denominator;
        self.remainder = product % denominator;
        let requested = u128::from(self.position.0) + delta;
        let next = requested.min(u128::from(available.0.max(self.position.0)));
        if next != requested {
            self.remainder = 0;
        }
        self.complete_advance(next, duration);
    }

    /// Audible and silent clocks cross the same loop boundary and invalidate old decoder histories.
    fn complete_advance(&mut self, next: u128, duration: MediaTime) {
        if let Some(span) = self.looping.filter(|span| span.end <= duration)
            && next >= u128::from(span.end.0)
        {
            self.position = MediaTime(
                span.start.0
                    + ((next - u128::from(span.start.0)) % u128::from(span.duration().0)) as u64,
            );
            self.remainder = 0;
            self.epoch = self.epoch.wrapping_add(1);
        } else {
            self.position = MediaTime(next.min(u128::from(duration.0)) as u64);
            if self.position >= duration {
                self.playing = false;
            }
        }
    }

    /// Steps to an exact output frame and pauses; stepping backwards is a real seek.
    pub fn step(
        &mut self,
        frames: i64,
        fps: FrameRate,
        duration: MediaTime,
    ) -> Result<(), StudioError> {
        let frame = i128::from(fps.frame_at(self.position)?) + i128::from(frames);
        let frame = frame.clamp(0, i128::from(fps.frame_at(duration)?)) as u64;
        self.pause();
        self.seek(fps.frame_time(frame)?, duration);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VisualSample {
    pub clip: Id,
    pub source: Id,
    pub time: MediaTime,
    pub track: u16,
    pub framing: Framing,
    /// Dissolve members add premultiplied contributions before their group is composited over lower tracks.
    pub mix: Option<Id>,
}

#[derive(Clone, Copy, Debug)]
pub struct AudioSample {
    pub clip: Id,
    pub source: Id,
    pub time: MediaTime,
    pub rate: f64,
    pub gain: f64,
}

/// The presentation edge reuses these vectors; no scene data or media bytes are cloned per frame.
#[derive(Default)]
pub struct FramePlan {
    pub time: MediaTime,
    pub visuals: Vec<VisualSample>,
    pub audio: Vec<AudioSample>,
    pub titles: Vec<usize>,
    pub camera: Option<CameraPose>,
    pub base_camera: Option<CameraPose>,
    pub shot: Option<Id>,
    pub spectated: Option<CameraTargetRef>,
    pub hide_hud: bool,
    pub hide_hand: bool,
    pub effects: EffectSample,
    transitions: Vec<usize>,
}

/// Rebuilt only when project metadata changes, with interval pruning for inactive tracks.
pub struct CompiledTimeline {
    project: Arc<Project>,
    index: IntervalIndex,
    visual_maps: Vec<TimeMap>,
    audio_maps: Vec<TimeMap>,
    clip_indices: BTreeMap<Id, usize>,
    camera_paths: Vec<Option<CompiledCameraPath>>,
    camera_blends: crate::camera::blend::CompiledCameraBlends,
}

impl CompiledTimeline {
    /// Validates the snapshot before constructing indexes consumed by the render/audio adapters.
    pub fn new(project: Arc<Project>) -> Result<Self, StudioError> {
        Self::new_reusing(project, None)
    }

    /// Rebinds metadata indexes while retaining unchanged positional rail integration from the prior document.
    pub fn new_reusing(
        project: Arc<Project>,
        previous: Option<&Self>,
    ) -> Result<Self, StudioError> {
        project.validate()?;
        let previous_paths: BTreeMap<_, _> = previous
            .into_iter()
            .flat_map(|old| {
                old.project
                    .shots
                    .iter()
                    .zip(&old.camera_paths)
                    .map(|(shot, path)| (shot.id, path.as_ref()))
            })
            .collect();
        let camera_paths = project
                .shots
                .iter()
                .map(|shot| {
                    CompiledCameraPath::new_reusing(
                        shot,
                        previous_paths.get(&shot.id).copied().flatten(),
                    )
                })
                .collect::<Vec<_>>();
        let visual_maps = project
            .clips
            .iter()
            .map(|clip| clip.time_map())
            .collect::<Result<Vec<_>, _>>()?;
        let camera_blends =
            crate::camera::blend::CompiledCameraBlends::new(&project, &visual_maps, &camera_paths)?;
        Ok(Self {
            index: IntervalIndex::new(&project)?,
            camera_paths,
            visual_maps,
            camera_blends,
            audio_maps: project
                .audio
                .iter()
                .map(|clip| clip.time_map())
                .collect::<Result<_, _>>()?,
            clip_indices: project
                .clips
                .iter()
                .enumerate()
                .map(|(index, clip)| (clip.id, index))
                .collect(),
            project,
        })
    }

    pub fn project(&self) -> &Arc<Project> {
        &self.project
    }

    /// Evaluates only active tracks. Actor lookup receives the selected replay source and source time.
    pub fn sample(
        &self,
        time: MediaTime,
        out: &mut FramePlan,
        actor: impl Fn(Id, CameraTargetRef, MediaTime) -> Option<ActorPose>,
    ) -> Result<(), StudioError> {
        out.time = time;
        out.visuals.clear();
        out.audio.clear();
        out.titles.clear();
        out.camera = None;
        out.base_camera = None;
        out.shot = None;
        out.spectated = None;
        out.hide_hud = !self.project.export.include_recorded_hud;
        out.hide_hand = false;
        out.effects = EffectSample::default();
        let mut shot = None;
        let mut hidden_visual = false;
        out.transitions.clear();
        self.index.visit(time, |slot| match slot {
            Slot::Clip(index) => {
                let clip = &self.project.clips[index];
                let muted = self
                    .project
                    .muted_tracks
                    .contains(&crate::tracks::Track::Video(clip.track));
                hidden_visual |= clip.enabled && muted;
                if clip.enabled
                    && !muted
                    && let Ok(source_time) = self.visual_maps[index].source_time(time)
                {
                    out.visuals.push(VisualSample {
                        clip: clip.id,
                        source: clip.source,
                        time: source_time,
                        track: clip.track,
                        framing: clip.framing,
                        mix: None,
                    });
                }
            }
            Slot::Camera(index) => {
                if self
                    .project
                    .muted_tracks
                    .contains(&crate::tracks::Track::Camera)
                {
                    return;
                }
                let candidate = &self.project.shots[index];
                if shot.is_none_or(|previous: usize| {
                    let previous = &self.project.shots[previous];
                    (candidate.span.start, candidate.id) > (previous.span.start, previous.id)
                }) {
                    shot = Some(index);
                }
            }
            Slot::Effect(index) => {
                let effect = &self.project.effects[index];
                let enabled =
                    effect
                        .origin
                        .and_then(|origin| origin.clip_id)
                        .and_then(|id| self.clip_indices.get(&id))
                        .is_none_or(|&index| {
                            self.project.clips[index].enabled
                                && !self.project.muted_tracks.contains(
                                    &crate::tracks::Track::Video(self.project.clips[index].track),
                                )
                        });
                if enabled
                    && !self
                        .project
                        .muted_tracks
                        .contains(&crate::tracks::Track::Effects)
                {
                    out.effects.add(effect.kind, effect.sample(time));
                }
            }
            Slot::Audio(index) => {
                let clip = &self.project.audio[index];
                let map = &self.audio_maps[index];
                let gain = clip.gain_in_span(time, map.span());
                if gain > 0.0
                    && !self.project.audio_muted(clip)
                    && let Ok(source_time) = map.source_time(time)
                {
                    out.audio.push(AudioSample {
                        clip: clip.id,
                        source: clip.source,
                        time: source_time,
                        rate: map.rate_at(time),
                        gain,
                    });
                }
            }
            Slot::Title(index) => {
                if !self
                    .project
                    .muted_tracks
                    .contains(&crate::tracks::Track::Titles)
                {
                    out.titles.push(index);
                }
            }
            Slot::Transition(index) => out.transitions.push(index),
        });
        let transitions = std::mem::take(&mut out.transitions);
        let transition_result = transitions
            .iter()
            .try_for_each(|&index| self.transition(index, time, out));
        out.transitions = transitions;
        transition_result?;
        out.effects.constrain();
        out.visuals
            .sort_unstable_by_key(|visual| (visual.track, visual.clip));
        if let Some(index) = shot.filter(|_| !(out.visuals.is_empty() && hidden_visual)) {
            let shot = &self.project.shots[index];
            out.shot = Some(shot.id);
            let selected = out.visuals.last();
            out.camera = shot.sample_compiled(
                time,
                |id| selected.and_then(|source| actor(source.source, id, source.time)),
                self.camera_paths[index].as_ref(),
            );
            out.spectated = shot.spectated_actor().filter(|_| out.camera.is_some());
            out.hide_hud |= shot.hide_hud;
            out.hide_hand = shot.hide_hand;
            self.blend_camera(index, time, out, &actor);
        }
        out.base_camera = out.camera;
        out.camera = out.camera.map(|pose| out.effects.camera_pose(pose, time));
        Ok(())
    }

    /// Crossfades retain two source layers; a dip changes presentation exposure without losing state.
    fn transition(
        &self,
        index: usize,
        time: MediaTime,
        out: &mut FramePlan,
    ) -> Result<(), StudioError> {
        let transition = &self.project.transitions[index];
        let from = self
            .project
            .clips
            .iter()
            .find(|clip| clip.id == transition.from)
            .ok_or(StudioError::Missing {
                kind: "clip",
                id: transition.from.0,
            })?;
        let to = self
            .project
            .clips
            .iter()
            .find(|clip| clip.id == transition.to)
            .ok_or(StudioError::Missing {
                kind: "clip",
                id: transition.to.0,
            })?;
        let start = transition.span_at(to.at)?.start.0;
        if !from.enabled
            || !to.enabled
            || self
                .project
                .muted_tracks
                .contains(&crate::tracks::Track::Video(from.track))
            || self
                .project
                .muted_tracks
                .contains(&crate::tracks::Track::Video(to.track))
        {
            return Ok(());
        }
        let progress =
            (time.0.saturating_sub(start) as f64 / transition.duration.0 as f64).clamp(0.0, 1.0);
        if transition.kind == TransitionKind::DipToBlack {
            out.effects.fade = out.effects.fade.max(1.0 - (2.0 * progress - 1.0).abs());
            return Ok(());
        }
        out.visuals
            .retain(|visual| visual.clip != from.id && visual.clip != to.id);
        for (clip, weight) in [(from, 1.0 - progress), (to, progress)] {
            let source = self
                .project
                .sources
                .iter()
                .find(|source| source.id == clip.source)
                .ok_or(StudioError::Missing {
                    kind: "source",
                    id: clip.source.0,
                })?;
            let mut framing = clip.framing;
            framing.opacity *= weight;
            out.visuals.push(VisualSample {
                clip: clip.id,
                source: clip.source,
                time: if source.kind == crate::model::MediaKind::Image {
                    MediaTime::ZERO
                } else {
                    self.visual_maps[self.clip_indices[&clip.id]]
                        .transition_time(time, source.duration)?
                },
                track: clip.track,
                framing,
                mix: Some(transition.id),
            });
        }
        Ok(())
    }
}
