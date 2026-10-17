//! Source-time speed authoring commits one shared video/audio/event clock per undoable edit.

use recording_studio::{
    effect::Easing,
    retime::{SpeedKey, SpeedRamp},
};

use super::*;

impl Studio {
    /// Speed ramps belong to the explicitly selected visual clip, never whichever source happens to be visible.
    fn speed_clip(&self) -> Result<Clip, String> {
        self.project()?
            .clips
            .iter()
            .find(|clip| Some(clip.id) == self.view.selected)
            .cloned()
            .ok_or_else(|| "Select a visual clip before editing its speed ramp.".into())
    }

    /// An exact stored source selection survives output-clock rounding; otherwise the playhead chooses the source.
    fn speed_source(&self, clip: &Clip) -> Result<MediaTime, String> {
        let map = clip.time_map().map_err(|error| error.to_string())?;
        if let Some((id, at)) = self.view.speed_key
            && id == clip.id
            && at >= clip.source_range.start
            && at <= clip.source_range.end
            && clip
                .speed_ramp
                .as_ref()
                .is_some_and(|ramp| ramp.key_at(at).is_some())
            && map.timeline_boundary(at).ok() == Some(self.transport.position())
        {
            return Ok(at);
        }
        if self.transport.position() < map.span().start
            || self.transport.position() > map.span().end
        {
            return Err(
                "Move the playhead inside the selected clip before adding a speed key.".into(),
            );
        }
        map.source_boundary(self.transport.position())
            .map_err(|error| error.to_string())
    }

    /// Keyboard/gamepad fallback exposes actual authored source keys and matching speed-edit intents.
    pub(super) fn speed_focus_actions(&self) -> Vec<Action> {
        let Ok(clip) = self.speed_clip() else {
            return Vec::new();
        };
        let mut actions = vec![Action::SetSpeedKey {
            rate: PlaybackRate::default(),
            ripple: false,
        }];
        if let Some(ramp) = &clip.speed_ramp {
            actions.extend([
                Action::DeleteSpeedKey { ripple: false },
                Action::CycleSpeedEasing { ripple: false },
                Action::ClearSpeedRamp { ripple: false },
                Action::StepSpeedKey(-1),
                Action::StepSpeedKey(1),
            ]);
            actions.extend(ramp.keys.iter().map(|key| Action::SelectSpeedKey {
                clip: clip.id,
                at: key.at,
            }));
        }
        actions
    }

    /// The first local ramp starts from 1x at both retained boundaries before adding the chosen multiplier.
    pub(super) fn set_speed_key(&mut self, rate: PlaybackRate, ripple: bool) -> Result<(), String> {
        rate.validate().map_err(|error| error.to_string())?;
        let clip = self.speed_clip()?;
        let at = self.speed_source(&clip)?;
        let ramp = clip.speed_ramp.clone().unwrap_or_else(|| SpeedRamp {
            keys: vec![
                SpeedKey {
                    at: clip.source_range.start,
                    rate: PlaybackRate::default(),
                    easing: Easing::Smooth,
                },
                SpeedKey {
                    at: clip.source_range.end,
                    rate: PlaybackRate::default(),
                    easing: Easing::Smooth,
                },
            ],
        });
        let ramp = ramp
            .keyed(SpeedKey {
                at,
                rate,
                easing: Easing::Smooth,
            })
            .map_err(|error| error.to_string())?;
        self.commit_speed_ramp(clip.id, Some(ramp), ripple, at)
    }

    /// Removing the last authored multiplier clears the optional ramp while keeping the overall clip rate.
    pub(super) fn delete_speed_key(&mut self, ripple: bool) -> Result<(), String> {
        let clip = self.speed_clip()?;
        let at = self.speed_source(&clip)?;
        let ramp = clip
            .speed_ramp
            .as_ref()
            .ok_or("This clip has no speed ramp.")?
            .without_key(at)
            .map_err(|error| error.to_string())?;
        self.commit_speed_ramp(clip.id, ramp, ripple, at)
    }

    /// Incoming easing changes the source segment arriving at the selected exact speed key.
    pub(super) fn cycle_speed_easing(&mut self, ripple: bool) -> Result<(), String> {
        let clip = self.speed_clip()?;
        let at = self.speed_source(&clip)?;
        let ramp = clip
            .speed_ramp
            .as_ref()
            .ok_or("This clip has no speed ramp.")?;
        let key = ramp
            .key_at(at)
            .ok_or("Select an exact speed key before changing easing.")?;
        let ramp = ramp
            .with_key_easing(at, key.easing.next())
            .map_err(|error| error.to_string())?;
        self.commit_speed_ramp(clip.id, Some(ramp), ripple, at)
    }

    /// Clearing a ramp restores constant-speed playback and keeps the same source at the playhead.
    pub(super) fn clear_speed_ramp(&mut self, ripple: bool) -> Result<(), String> {
        let clip = self.speed_clip()?;
        let at = self.speed_source(&clip)?;
        self.commit_speed_ramp(clip.id, None, ripple, at)
    }

    /// Source keys remain selected exactly even when their output position lies between timeline microseconds.
    pub(super) fn select_speed_key(&mut self, id: Id, source: MediaTime) -> Result<(), String> {
        let project = self.project()?;
        let clip = project
            .clips
            .iter()
            .find(|clip| clip.id == id)
            .ok_or("This visual clip is no longer present.")?;
        clip.speed_ramp
            .as_ref()
            .and_then(|ramp| ramp.key_at(source))
            .ok_or("This speed key is no longer present.")?;
        if source < clip.source_range.start || source > clip.source_range.end {
            return Err("This speed key is outside the retained source interval. Extend the trim to edit it.".into());
        }
        let at = clip
            .time_map()
            .and_then(|map| map.timeline_boundary(source))
            .map_err(|error| error.to_string())?;
        let duration = project.duration();
        self.select_timeline_item(id)?;
        self.cancel_camera_value();
        self.transport.pause();
        self.transport.seek(at, duration);
        Arc::make_mut(&mut self.view).speed_key = Some((id, source));
        Ok(())
    }

    /// Previous/next visits retained source keys in order, without landing on keys hidden beyond a trim.
    pub(super) fn step_speed_key(&mut self, direction: i8) -> Result<(), String> {
        let clip = self.speed_clip()?;
        let at = self.speed_source(&clip)?;
        let ramp = clip
            .speed_ramp
            .as_ref()
            .ok_or("This clip has no speed ramp.")?;
        let mut keys = ramp
            .keys
            .iter()
            .filter(|key| key.at >= clip.source_range.start && key.at <= clip.source_range.end);
        let next = if direction < 0 {
            keys.rev().find(|key| key.at < at)
        } else {
            keys.find(|key| key.at > at)
        }
        .ok_or("There is no further speed key in that direction.")?;
        self.select_speed_key(clip.id, next.at)
    }

    /// Video, linked game audio and generated event metadata update in the same validated transaction.
    fn commit_speed_ramp(
        &mut self,
        id: Id,
        ramp: Option<SpeedRamp>,
        ripple: bool,
        source: MediaTime,
    ) -> Result<(), String> {
        self.transport.pause();
        self.edit(
            "Change speed ramp",
            &[Command::SetSpeedRamp { id, ramp, ripple }],
        )?;
        let project = self.project()?;
        let clip = project
            .clips
            .iter()
            .find(|clip| clip.id == id)
            .ok_or("This visual clip is no longer present.")?;
        let map = clip.time_map().map_err(|error| error.to_string())?;
        let at = map
            .timeline_boundary(source)
            .map_err(|error| error.to_string())?;
        let key = clip
            .speed_ramp
            .as_ref()
            .and_then(|ramp| ramp.key_at(source))
            .map(|_| (id, source));
        let duration = project.duration();
        self.transport.seek(at, duration);
        Arc::make_mut(&mut self.view).speed_key = key;
        Ok(())
    }
}
