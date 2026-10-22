//! Subject search and paging are transient inspection preferences, never camera retarget commands.

use super::*;
use launcher::menu::studio::{
    CAMERA_SUBJECT_ROWS, CameraSubjectResults, CameraSubjectsAction as S,
};
use recording_studio::camera::CameraSubjectSearch;

impl Studio {
    /// Entering search pauses inspection and leaves project metadata and captured spawn bindings untouched.
    pub(in crate::menu) fn begin_camera_subject_search(&mut self) {
        let text = self.view.camera_workspace.subject_search.text();
        if self.subject_text.as_str() != text {
            self.subject_text.set_text(text);
        }
        self.transport.pause();
        self.release_camera();
    }

    /// Compile the input once per text change; result publication shares immutable actor data.
    pub(in crate::menu) fn publish_camera_subject_search(&mut self) {
        match CameraSubjectSearch::new(self.subject_text.as_str()) {
            Ok(query) => {
                if self.view.camera_workspace.subject_search != query {
                    let workspace = &mut Arc::make_mut(&mut self.view).camera_workspace;
                    workspace.subject_search = query;
                    workspace.subject_first = 0;
                }
                self.reconcile_camera_subjects();
            }
            Err(error) => Arc::make_mut(&mut self.view).error = Some(error.to_string()),
        }
    }

    /// Clear restores the class-filtered browser; paging preserves the exact selected spawn reference.
    pub(in crate::menu::studio) fn camera_subjects_action(
        &mut self,
        action: S,
    ) -> Result<(), String> {
        match action {
            S::Search => self.begin_camera_subject_search(),
            S::Done => {}
            S::Clear => {
                self.subject_text.set_text("");
                self.publish_camera_subject_search();
            }
            S::Select {
                source,
                epoch,
                target,
            } => {
                let preview = self.displayed_camera_preview()?;
                if preview.source != source || preview.epoch != epoch {
                    return Err("This subject result belongs to another recorded frame. Choose it again from the current results.".into());
                }
                self.select_camera_subject(target)?;
            }
            S::Page(direction) => {
                self.reconcile_camera_subjects();
                let workspace = &mut Arc::make_mut(&mut self.view).camera_workspace;
                if let Some(results) = &workspace.subject_results {
                    let first = results.range(workspace.subject_first).start;
                    let next = if direction < 0 {
                        first.saturating_sub(CAMERA_SUBJECT_ROWS)
                    } else {
                        first.saturating_add(CAMERA_SUBJECT_ROWS)
                    };
                    workspace.subject_first = results.range(next).start;
                }
            }
        }
        Ok(())
    }

    /// An unchanged target publication/query/filter reuses the same index buffer with no actor-array scan.
    pub(in crate::menu::studio) fn reconcile_camera_subjects(&mut self) {
        let workspace = &self.view.camera_workspace;
        let Some(preview) = &self.view.preview.camera else {
            if workspace.subject_results.is_some() {
                let workspace = &mut Arc::make_mut(&mut self.view).camera_workspace;
                workspace.subject_results = None;
                workspace.subject_first = 0;
            }
            return;
        };
        if workspace.subject_results.as_ref().is_some_and(|results| {
            results.is_current(preview, &workspace.subject_search, workspace.subject_filter)
        }) {
            return;
        }
        let results =
            CameraSubjectResults::new(preview, &workspace.subject_search, workspace.subject_filter);
        let first = results.range(workspace.subject_first).start;
        let workspace = &mut Arc::make_mut(&mut self.view).camera_workspace;
        workspace.subject_results = Some(results);
        workspace.subject_first = first;
    }

    /// Focus lists contain only the same bounded matching rows as the painted subject browser.
    pub(super) fn camera_subject_focus_actions(&self) -> Vec<Action> {
        let workspace = &self.view.camera_workspace;
        let mut actions = vec![
            Action::CameraSubjects(S::Search),
            Action::ToggleCameraSubjectFilter,
        ];
        if !workspace.subject_search.text().is_empty() {
            actions.push(Action::CameraSubjects(S::Clear));
        }
        if !self.view.preview.ready {
            return actions;
        }
        let Some(results) = &workspace.subject_results else {
            return actions;
        };
        if !results.is_empty() {
            actions.extend([Action::CycleCameraTarget(-1), Action::CycleCameraTarget(1)]);
        }
        if !workspace.subject_search.is_empty() {
            let range = results.range(workspace.subject_first);
            for index in range.clone() {
                if let Some(target) = results.target(index)
                    && let Some(preview) = &self.view.preview.camera
                {
                    actions.push(Action::CameraSubjects(S::Select {
                        source: preview.source,
                        epoch: preview.epoch,
                        target: target.reference(),
                    }));
                }
            }
            if range.start > 0 {
                actions.push(Action::CameraSubjects(S::Page(-1)));
            }
            if range.end < results.len() {
                actions.push(Action::CameraSubjects(S::Page(1)));
            }
        }
        actions
    }
}

#[cfg(test)]
mod tests;
