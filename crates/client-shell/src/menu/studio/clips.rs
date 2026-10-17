//! Source framing, visual layers and adjacent-cut transitions are ordinary reversible edits.

use super::*;
use launcher::menu::studio::ClipTimeField;
use recording_studio::model::{Transition, TransitionKind};

mod values;

pub(super) struct ClipTimeEdit {
    document: u64,
    revision: u64,
    clip: Id,
    field: ClipTimeField,
}

impl Studio {
    /// Fallback navigation exposes numeric fields only for a selected visual clip.
    pub(super) fn clip_focus_actions(&self) -> Vec<Action> {
        if !self.project().is_ok_and(|project| {
            project
                .clips
                .iter()
                .any(|clip| Some(clip.id) == self.view.selected)
        }) {
            return Vec::new();
        }
        let mut actions: Vec<_> = ClipTimeField::ALL
            .iter()
            .copied()
            .map(Action::EditClipTime)
            .collect();
        if self.view.clip_draft.is_some() {
            actions.extend([Action::ApplyClipTime, Action::CancelClipTime]);
        }
        actions
    }

    /// A clip inspector never treats a camera/effect/media-source selection as a visual clip.
    fn selected_visual(&self) -> Result<Clip, String> {
        self.project()?
            .clips
            .iter()
            .find(|clip| Some(clip.id) == self.view.selected)
            .cloned()
            .ok_or_else(|| "Select a video or replay clip first.".into())
    }

    /// Adjusts only source crop/opacity and keeps source time, camera and linked audio unchanged.
    pub(super) fn adjust_framing(
        &mut self,
        parameter: recording_studio::timeline::FramingParameter,
        direction: i8,
    ) -> Result<(), String> {
        let clip = self.selected_visual()?;
        self.edit(
            "Change clip framing",
            &[Command::SetFraming {
                id: clip.id,
                framing: parameter.adjusted(clip.framing, direction),
            }],
        )
    }

    /// Reset uses the owning domain default rather than another hand-maintained crop rectangle.
    pub(super) fn reset_framing(&mut self) -> Result<(), String> {
        let clip = self.selected_visual()?;
        self.edit(
            "Reset clip framing",
            &[Command::SetFraming {
                id: clip.id,
                framing: Framing::default(),
            }],
        )
    }

    /// Layer admission rejects overlapping clips on the destination track without changing the project.
    pub(super) fn move_clip_layer(&mut self, direction: i8) -> Result<(), String> {
        let clip = self.selected_visual()?;
        let track = (i32::from(clip.track) + i32::from(direction))
            .clamp(0, i32::from(recording_studio::model::MAX_VIDEO_TRACKS) - 1)
            as u16;
        self.edit(
            "Change clip layer",
            &[Command::MoveClip {
                id: clip.id,
                at: clip.at,
                track,
            }],
        )
    }

    /// A transition belongs to the adjacent cut on the same visual layer, with duration bounded by both clips.
    pub(super) fn transition_to_next(&mut self, kind: TransitionKind) -> Result<(), String> {
        let clip = self.selected_visual()?;
        let project = self.project()?;
        let span = clip.span().map_err(|error| error.to_string())?;
        let next = project.clips.iter().find(|next| next.enabled && next.track == clip.track && next.at == span.end)
            .ok_or("Place another clip immediately after this clip on the same layer before adding a transition.")?;
        let duration = MediaTime(1_000_000)
            .min(span.duration())
            .min(next.span().map_err(|error| error.to_string())?.duration());
        let old = project
            .transitions
            .iter()
            .find(|transition| transition.from == clip.id);
        let id = old
            .map(|item| item.id)
            .unwrap_or(project.next_id().map_err(|error| error.to_string())?);
        let transition = Transition {
            id,
            from: clip.id,
            to: next.id,
            duration,
            kind,
        };
        let mut commands = Vec::new();
        if let Some(old) = old {
            commands.push(Command::Delete { id: old.id });
        }
        commands.push(Command::AddTransition { transition });
        self.edit("Change cut transition", &commands)
    }

    /// Removes only the selected clip's outgoing transition, leaving the media and adjacent clip intact.
    pub(super) fn remove_clip_transition(&mut self) -> Result<(), String> {
        let clip = self.selected_visual()?;
        let id = self
            .project()?
            .transitions
            .iter()
            .find(|item| item.from == clip.id)
            .map(|item| item.id)
            .ok_or("This clip has no outgoing transition.")?;
        self.edit("Remove cut transition", &[Command::Delete { id }])
    }
}
