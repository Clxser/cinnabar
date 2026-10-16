//! Native effect edits use the same transactional commands for manual and combat-generated curves.

use super::*;
use recording_studio::effect::{Curve, Easing, EffectClip, EffectKind};

impl Studio {
    /// Focus fallback follows the currently selected effect instead of indexing every curve key.
    pub(super) fn effect_focus_actions(&self) -> Vec<Action> {
        let mut actions = EffectKind::ALL
            .iter()
            .copied()
            .map(Action::AddEffect)
            .collect::<Vec<_>>();
        if self.selected_effect().is_ok() {
            actions.extend([
                Action::ToggleEffect,
                Action::AdjustEffect(-1),
                Action::AdjustEffect(1),
                Action::AddEffectKey,
                Action::DeleteEffectKey,
                Action::CycleEffectEasing,
                Action::StepEffectKey(-1),
                Action::StepEffectKey(1),
                Action::MoveEffect(-1),
                Action::MoveEffect(1),
            ]);
        }
        actions
    }

    /// Manual effects retain no combat attribution; generated effects retain their source evidence through edits.
    pub(super) fn add_effect(&mut self, kind: EffectKind) -> Result<(), String> {
        let project = self.project()?;
        let duration = project.duration();
        if duration == MediaTime::ZERO {
            return Err("Add a clip before adding effects.".into());
        }
        let start = MediaTime(self.transport.position().0.min(duration.0 - 1));
        let start = project
            .export
            .fps
            .frame_time(
                project
                    .export
                    .fps
                    .frame_at(start)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
        let span = Span::new(
            start,
            MediaTime(duration.0.min(start.0.saturating_add(2_000_000))),
        )
        .map_err(|error| error.to_string())?;
        let id = project.next_id().map_err(|error| error.to_string())?;
        let effect = EffectClip {
            id,
            span,
            kind,
            amount: Curve::constant(kind.initial()),
            enabled: true,
            origin: None,
        };
        self.transport.pause();
        self.transport.seek(start, duration);
        self.edit("Add effect", &[Command::AddEffect { effect }])?;
        Arc::make_mut(&mut self.view).selected = Some(id);
        Ok(())
    }

    /// Selection, rather than nearest timestamp, determines which effect is edited.
    fn selected_effect(&self) -> Result<&EffectClip, String> {
        self.project()?
            .effects
            .iter()
            .find(|effect| Some(effect.id) == self.view.selected)
            .ok_or_else(|| "Select an effect on the timeline first.".into())
    }

    /// Disable is reversible and preserves the full curve and event provenance.
    pub(super) fn toggle_effect(&mut self) -> Result<(), String> {
        let mut effect = self.selected_effect()?.clone();
        effect.enabled = !effect.enabled;
        self.edit("Toggle effect", &[Command::ReplaceEffect { effect }])
    }

    /// The shared fine-adjustment preference changes step size, never the selected key identity.
    pub(super) fn adjust_effect(&mut self, direction: i8) -> Result<(), String> {
        let steps = f64::from(direction.signum()) * if self.view.camera_fine { 0.1 } else { 1.0 };
        let effect = self
            .selected_effect()?
            .adjusted(self.transport.position(), steps)
            .map_err(|error| error.to_string())?;
        self.transport.pause();
        self.edit("Adjust effect key", &[Command::ReplaceEffect { effect }])
    }

    /// A key captures the current sampled value without depending on preview-worker completion.
    pub(super) fn key_effect(&mut self) -> Result<(), String> {
        let effect = self.selected_effect()?;
        let at = self.transport.position();
        let value = effect.amount.sample(at.saturating_sub(effect.span.start));
        let effect = effect.keyed(at, value).map_err(|error| error.to_string())?;
        self.transport.pause();
        self.edit("Add effect key", &[Command::ReplaceEffect { effect }])
    }

    /// End-point keys remain editable; the last key cannot be removed.
    pub(super) fn delete_effect_key(&mut self) -> Result<(), String> {
        let effect = self
            .selected_effect()?
            .without_key(self.transport.position())
            .map_err(|error| error.to_string())?;
        self.transport.pause();
        self.edit("Delete effect key", &[Command::ReplaceEffect { effect }])
    }

    /// Changing easing creates an exact-playhead key when needed, leaving neighbours intact.
    pub(super) fn cycle_effect_easing(&mut self) -> Result<(), String> {
        let effect = self.selected_effect()?;
        let at = self.transport.position();
        let offset = at.saturating_sub(effect.span.start);
        let easing = effect
            .amount
            .keys
            .iter()
            .find(|key| key.at == offset)
            .map_or(Easing::Smooth, |key| key.easing)
            .next();
        let effect = effect
            .eased(at, easing)
            .map_err(|error| error.to_string())?;
        self.transport.pause();
        self.edit("Change effect easing", &[Command::ReplaceEffect { effect }])
    }

    /// Key navigation wraps through exact authored timestamps, including a boundary key at the clip end.
    pub(super) fn step_effect_key(&mut self, direction: i8) -> Result<(), String> {
        let effect = self.selected_effect()?;
        let offset = self.transport.position().saturating_sub(effect.span.start);
        let keys = &effect.amount.keys;
        let key = if direction < 0 {
            keys.iter()
                .rev()
                .find(|key| key.at < offset)
                .or_else(|| keys.last())
        } else {
            keys.iter()
                .find(|key| key.at > offset)
                .or_else(|| keys.first())
        }
        .ok_or("This effect has no keys.")?;
        let at = effect
            .span
            .start
            .checked_add(key.at)
            .map_err(|error| error.to_string())?;
        let duration = self.project()?.duration();
        self.transport.pause();
        self.transport.seek(at, duration);
        Ok(())
    }

    /// Moving an effect retains relative curve timing and its original combat evidence.
    pub(super) fn move_effect(&mut self, frames: i8) -> Result<(), String> {
        let mut effect = self.selected_effect()?.clone();
        let delta = self
            .project()?
            .export
            .fps
            .frame_time(u64::from(frames.unsigned_abs()))
            .map_err(|error| error.to_string())?;
        let start = if frames < 0 {
            effect.span.start.saturating_sub(delta)
        } else {
            effect
                .span
                .start
                .checked_add(delta)
                .map_err(|error| error.to_string())?
        };
        effect.span = Span::new(
            start,
            start
                .checked_add(effect.span.duration())
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        self.transport.pause();
        self.edit("Move effect", &[Command::ReplaceEffect { effect }])?;
        let duration = self.project()?.duration();
        self.transport.seek(start, duration);
        Ok(())
    }

    /// Timeline hits carry immutable key time and effect identity, so edits cannot redirect a stale index.
    pub(super) fn select_effect_key(&mut self, id: Id, at: MediaTime) -> Result<(), String> {
        let project = self.project()?;
        let effect = project
            .effects
            .iter()
            .find(|effect| effect.id == id)
            .ok_or("This effect no longer exists.")?;
        effect
            .amount
            .keys
            .binary_search_by_key(&at, |key| key.at)
            .map_err(|_| "This effect key no longer exists.")?;
        let position = effect
            .span
            .start
            .checked_add(at)
            .map_err(|error| error.to_string())?;
        let duration = project.duration();
        self.transport.pause();
        self.transport.seek(position, duration);
        Arc::make_mut(&mut self.view).selected = Some(id);
        Ok(())
    }
}
