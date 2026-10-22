//! Native selection and track controls share one transactional project policy.
use super::*;
use launcher::menu::studio::timeline::TimelineAction;
use std::collections::BTreeSet;

impl Studio {
    /// A removed item never remains an invisible selection after undo, redo or dependent cleanup.
    pub(in super::super) fn reconcile_timeline_selection(&mut self) {
        let Some(project) = self.editor.as_ref().map(Editor::project) else {
            return;
        };
        if self
            .view
            .project
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, project))
        {
            let view = Arc::make_mut(&mut self.view);
            if let Some(id) = view
                .selected
                .filter(|id| !view.workspace.timeline.selected.contains(id))
                && recording_studio::tracks::item_track(project, id).is_ok()
            {
                view.workspace.timeline.selected.clear();
                view.workspace.timeline.selected.insert(id);
            }
            return;
        }
        let ids = recording_studio::timeline::selection_ids(project)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let view = Arc::make_mut(&mut self.view);
        view.workspace
            .timeline
            .selected
            .retain(|id| ids.contains(id));
        if let Some(id) = view.selected.filter(|id| ids.contains(id)) {
            if !view.workspace.timeline.selected.contains(&id) {
                view.workspace.timeline.selected.clear();
                view.workspace.timeline.selected.insert(id);
            }
        } else if view
            .selected
            .is_some_and(|id| !project.sources.iter().any(|source| source.id == id))
        {
            view.selected = view.workspace.timeline.selected.iter().next().copied();
        }
    }

    /// Pointer modifiers change timeline selection without altering undo history or source media.
    pub(super) fn select_group(
        &mut self,
        id: Id,
        extend: bool,
        toggle: bool,
    ) -> Result<bool, String> {
        let project = self.project()?.clone();
        let order = recording_studio::timeline::selection_ids(&project);
        let timeline = &mut Arc::make_mut(&mut self.view).workspace.timeline;
        if toggle {
            if !timeline.selected.remove(&id) {
                timeline.selected.insert(id);
            }
        } else if extend {
            let first = timeline
                .anchor
                .and_then(|anchor| order.iter().position(|id| *id == anchor));
            let last = order.iter().position(|candidate| *candidate == id);
            if let (Some(first), Some(last)) = (first, last) {
                timeline
                    .selected
                    .extend(order[first.min(last)..=first.max(last)].iter().copied());
            } else {
                timeline.selected.insert(id);
            }
        } else if !timeline.selected.contains(&id) {
            timeline.selected.clear();
            timeline.selected.insert(id);
        }
        if !extend {
            timeline.anchor = Some(id);
        }
        if !timeline.selected.contains(&id) {
            let primary = timeline.selected.iter().next().copied();
            Arc::make_mut(&mut self.view).selected = primary;
            return Ok(false);
        }
        self.select_timeline_item(id)?;
        Ok(true)
    }

    /// Selection operations remain atomic and locks apply to synchronized linked children too.
    pub(in super::super) fn timeline_action(
        &mut self,
        action: TimelineAction,
    ) -> Result<(), String> {
        match action {
            TimelineAction::SelectAll => {
                let ids = recording_studio::timeline::selection_ids(self.project()?)
                    .into_iter()
                    .filter(|id| {
                        recording_studio::tracks::item_track(self.project().unwrap(), *id)
                            .is_ok_and(|track| {
                                !self.view.workspace.timeline.locked.contains(&track)
                            })
                    })
                    .collect::<BTreeSet<_>>();
                let view = Arc::make_mut(&mut self.view);
                view.selected = ids.iter().next().copied();
                view.workspace.timeline.selected = ids;
            }
            TimelineAction::Duplicate => {
                let selected = self.timeline_selection();
                let (commands, ids) =
                    recording_studio::timeline::duplicate_selection(self.project()?, &selected)
                        .map_err(|error| error.to_string())?;
                self.edit("Duplicate selection", &commands)?;
                let view = Arc::make_mut(&mut self.view);
                view.selected = ids.iter().next().copied();
                view.workspace.timeline.selected = ids;
            }
            TimelineAction::ToggleSnapping => {
                let timeline = &mut Arc::make_mut(&mut self.view).workspace.timeline;
                timeline.snapping = !timeline.snapping;
            }
            TimelineAction::ToggleLock(track) => {
                let locked = &mut Arc::make_mut(&mut self.view).workspace.timeline.locked;
                if !locked.remove(&track) {
                    locked.insert(track);
                }
            }
            TimelineAction::ToggleMute(track) => {
                let muted = !self.project()?.muted_tracks.contains(&track);
                self.edit(
                    "Mute timeline track",
                    &[Command::SetTrackMuted { track, muted }],
                )?;
            }
            TimelineAction::DragKey { .. } => {}
        }
        Ok(())
    }

    /// Inspector primary selection participates when no explicit group has been selected yet.
    pub(super) fn timeline_selection(&self) -> BTreeSet<Id> {
        let mut ids = self.view.workspace.timeline.selected.clone();
        if ids.is_empty() {
            ids.extend(self.view.selected);
        }
        ids
    }

    /// One delete removes a group and lets the domain own linked cleanup and source anchors.
    pub(in super::super) fn delete_timeline_selection(&mut self) -> Result<(), String> {
        let commands = recording_studio::timeline::delete_selection(
            self.project()?,
            &self.timeline_selection(),
        )
        .map_err(|error| error.to_string())?;
        self.edit("Delete selection", &commands)?;
        let view = Arc::make_mut(&mut self.view);
        view.selected = None;
        view.workspace.timeline.selected.clear();
        Ok(())
    }

    /// Cancellation removes drag overlays without applying any project metadata.
    pub(in super::super) fn clear_timeline_drag(&mut self) {
        let timeline = &mut Arc::make_mut(&mut self.view).workspace.timeline;
        timeline.ghost.clear();
        timeline.snap_line = None;
        timeline.key_drag = None;
    }
}
