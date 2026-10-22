//! Explicit adjacent-rig entry transitions share deterministic, source-safe preview/export evaluation.

use super::*;
use crate::{Project, model::MediaKind, retime::TimeMap};
use glam::{DQuat, EulerRot};
use std::collections::{BTreeMap, BTreeSet};

mod motion;
use motion::TerminalMotion;

/// A saved opt-in transition uses the existing easing registry; absence means an exact legacy cut.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraBlend {
    pub duration: MediaTime,
    pub easing: Easing,
}

impl CameraBlend {
    pub const DEFAULT_DURATION: MediaTime = MediaTime(500_000);
    pub const DURATION_STEP: MediaTime = MediaTime(250_000);

    /// The entry interval fits its own shot; even one-microsecond shots remain representable.
    pub fn validate(self, shot_duration: MediaTime) -> Result<(), StudioError> {
        if self.duration == MediaTime::ZERO || self.duration > shot_duration {
            return Err(StudioError::Invalid(
                "camera entry blend must be nonzero and fit its shot",
            ));
        }
        Ok(())
    }

    /// Interpolates world position and lens, with a shortest quaternion arc in the renderer's YXZ convention.
    pub fn sample(
        self,
        outgoing: CameraPose,
        incoming: CameraPose,
        elapsed: MediaTime,
    ) -> CameraPose {
        if elapsed >= self.duration {
            return incoming;
        }
        let t = self
            .easing
            .sample(elapsed.0 as f64 / self.duration.0 as f64);
        if t <= 0.0 {
            return outgoing;
        }
        if t >= 1.0 {
            return incoming;
        }
        let rotation = orientation(outgoing)
            .slerp(orientation(incoming), t)
            .normalize();
        with_orientation(
            CameraPose {
                position: DVec3::from_array(outgoing.position)
                    .lerp(DVec3::from_array(incoming.position), t)
                    .to_array(),
                fov: outgoing.fov + (incoming.fov - outgoing.fov) * t,
                ..incoming
            },
            rotation,
        )
    }
}

impl CameraShot {
    /// Replaces only entry metadata; the ordinary shot replacement command supplies undo and persistence.
    pub fn with_blend_in(&self, blend: Option<CameraBlend>) -> Result<Self, StudioError> {
        if let Some(blend) = blend {
            blend.validate(self.span.duration())?;
        }
        let mut next = self.clone();
        next.blend_in = blend;
        next.validate()?;
        Ok(next)
    }
}

/// Quaternion conversion matches the renderer, including Bedrock's positive-down pitch and yaw sign.
fn orientation(pose: CameraPose) -> DQuat {
    DQuat::from_euler(
        EulerRot::YXZ,
        (180.0 - pose.yaw.rem_euclid(360.0)).to_radians(),
        -pose.pitch.to_radians(),
        pose.roll.rem_euclid(360.0).to_radians(),
    )
}

/// Quaternion poses return to the same network-facing degree convention as other rig samplers.
fn with_orientation(mut pose: CameraPose, rotation: DQuat) -> CameraPose {
    let (yaw, pitch, roll) = rotation.to_euler(EulerRot::YXZ);
    pose.yaw = 180.0 - yaw.to_degrees();
    pose.pitch = (-pitch.to_degrees()).clamp(-90.0, 90.0);
    pose.roll = roll.to_degrees();
    pose
}

#[derive(Clone, Copy)]
pub(crate) struct BlendPlan {
    pub previous: usize,
    pub clip: usize,
    pub end: MediaTime,
    pub motion: TerminalMotion,
}

/// One immutable plan per incoming shot; frame sampling performs no project walk or heap work.
pub(crate) struct CompiledCameraBlends(Vec<Option<BlendPlan>>);

#[derive(Clone, Copy)]
struct Run {
    start: MediaTime,
    end: MediaTime,
    owner: Option<usize>,
}

impl CompiledCameraBlends {
    /// A sweep compiles exact shot priority and one continuous unambiguous replay clip at edit time.
    pub(crate) fn new(
        project: &Project,
        maps: &[TimeMap],
        paths: &[Option<CompiledCameraPath>],
    ) -> Result<Self, StudioError> {
        if project.muted_tracks.contains(&crate::tracks::Track::Camera)
            || project.shots.iter().all(|shot| shot.blend_in.is_none())
        {
            return Ok(Self(Vec::new()));
        }
        let mut order: Vec<_> = (0..project.shots.len()).collect();
        order.sort_unstable_by_key(|&index| {
            (project.shots[index].span.start, project.shots[index].id)
        });
        let events = order
            .iter()
            .enumerate()
            .flat_map(|(rank, &index)| {
                let span = project.shots[index].span;
                [(span.start, true, rank), (span.end, false, rank)]
            })
            .collect();
        let shots = runs(events, |active| active.last().map(|&rank| order[rank]));
        let kinds: BTreeMap<_, _> = project
            .sources
            .iter()
            .map(|source| (source.id, source.kind))
            .collect();
        let mut events: Vec<_> = project
            .clips
            .iter()
            .enumerate()
            .filter(|(_, clip)| {
                clip.enabled
                    && !project
                        .muted_tracks
                        .contains(&crate::tracks::Track::Video(clip.track))
            })
            .flat_map(|(index, _)| {
                let span = maps[index].span();
                [(span.start, true, index), (span.end, false, index)]
            })
            .collect();
        let clips: BTreeMap<_, _> = project
            .clips
            .iter()
            .enumerate()
            .map(|(index, clip)| (clip.id, index))
            .collect();
        for (index, transition) in project.transitions.iter().enumerate() {
            let from = clips[&transition.from];
            let to = clips[&transition.to];
            if !project.clips[from].enabled
                || !project.clips[to].enabled
                || project
                    .muted_tracks
                    .contains(&crate::tracks::Track::Video(project.clips[from].track))
            {
                continue;
            }
            let span = transition.span_at(project.clips[to].at)?;
            let token = project.clips.len() + index;
            events.extend([(span.start, true, token), (span.end, false, token)]);
        }
        let sources = runs(events, |active| {
            let &index = active.first().filter(|_| active.len() == 1)?;
            let clip = project.clips.get(index)?;
            (kinds.get(&clip.source) == Some(&MediaKind::Replay)).then_some(index)
        });
        let plans = project
            .shots
            .iter()
            .enumerate()
            .map(|(index, shot)| {
                let blend = shot.blend_in?;
                let before = MediaTime(shot.span.start.0.checked_sub(1)?);
                let end = shot.span.start.checked_add(blend.duration).ok()?;
                let incoming = run_at(&shots, shot.span.start)?;
                let previous = run_at(&shots, before)?.owner?;
                let source = run_at(&sources, before)?;
                if incoming.owner != Some(index)
                    || incoming.end < end
                    || project.shots[previous].span.end != shot.span.start
                    || source.end < end
                    || !bound_target(shot)
                    || !bound_target(&project.shots[previous])
                {
                    return None;
                }
                Some(BlendPlan {
                    previous,
                    clip: source.owner?,
                    end,
                    motion: TerminalMotion::new(&project.shots[previous], paths[previous].as_ref()),
                })
            })
            .collect();
        Ok(Self(plans))
    }

    /// A missing or no-longer-compatible neighbor is an exact cut, never fabricated camera data.
    pub(crate) fn get(&self, index: usize, time: MediaTime) -> Option<BlendPlan> {
        self.0
            .get(index)
            .copied()
            .flatten()
            .filter(|plan| time < plan.end)
    }
}

/// New blends require spawn identity for subject rigs; legacy unbound rigs keep their original cut behavior.
fn bound_target(shot: &CameraShot) -> bool {
    shot.target_reference()
        .is_none_or(|target| target.lifetime.is_some())
}

/// Sorted start/end events produce continuous owner runs without quadratic pair comparisons.
fn runs(
    mut events: Vec<(MediaTime, bool, usize)>,
    owner: impl Fn(&BTreeSet<usize>) -> Option<usize>,
) -> Vec<Run> {
    events.sort_unstable();
    let mut result: Vec<Run> = Vec::new();
    let mut active = BTreeSet::new();
    let mut cursor = 0;
    while cursor < events.len() {
        let start = events[cursor].0;
        while cursor < events.len() && events[cursor].0 == start {
            let (_, enter, token) = events[cursor];
            if enter {
                active.insert(token);
            } else {
                active.remove(&token);
            }
            cursor += 1;
        }
        let Some(&(end, _, _)) = events.get(cursor) else {
            break;
        };
        let selected = owner(&active);
        if let Some(last) = result
            .last_mut()
            .filter(|last| last.end == start && last.owner == selected)
        {
            last.end = end;
        } else {
            result.push(Run {
                start,
                end,
                owner: selected,
            });
        }
    }
    result
}

/// Timeline lookups are bounded logarithmic searches over compiled ownership boundaries.
fn run_at(runs: &[Run], time: MediaTime) -> Option<&Run> {
    runs.get(
        runs.partition_point(|run| run.start <= time)
            .checked_sub(1)?,
    )
    .filter(|run| time < run.end)
}

#[cfg(test)]
mod tests;
