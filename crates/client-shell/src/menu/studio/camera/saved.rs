//! Saved-view drafts are explicit transactions; inspecting and recalling never edit the project.

use super::*;
use launcher::menu::studio::{CameraViewNameDraft, CameraViewsAction as V};
use recording_studio::camera::SavedCameraView;

pub(in crate::menu::studio) struct CameraViewEdit {
    document: u64,
    revision: u64,
    original: Option<SavedCameraView>,
    view: SavedCameraView,
}

impl Studio {
    /// Typed actions keep source binding and stable IDs independent of pagination or display names.
    pub(in crate::menu::studio) fn camera_views_action(&mut self, action: V) -> Result<(), String> {
        match action {
            V::Section(section) => {
                self.cancel_camera_view_name();
                Arc::make_mut(&mut self.view).camera_workspace.tools = section;
                Ok(())
            }
            V::New => self.begin_camera_view_name(None),
            V::Rename(id) => self.begin_camera_view_name(Some(id)),
            V::Save => self.save_camera_view_name(),
            V::Cancel => {
                self.cancel_camera_view_name();
                Ok(())
            }
            V::Select(id) => {
                self.saved_camera_view(id)?;
                self.cancel_camera_view_name();
                Arc::make_mut(&mut self.view)
                    .camera_workspace
                    .saved_views
                    .selected = Some(id);
                Ok(())
            }
            V::Recall(id) => {
                let view = self.saved_camera_view(id)?;
                let preview = self.camera_preview()?;
                if !preview.interactive {
                    return Err("Saved views require one unframed 3D replay view.".into());
                }
                let pose = view
                    .recall(self.project()?, preview.source)
                    .map_err(|error| error.to_string())?;
                self.install_inspection_view(pose)?;
                Arc::make_mut(&mut self.view)
                    .camera_workspace
                    .saved_views
                    .selected = Some(id);
                Ok(())
            }
            V::Delete(id) => {
                self.saved_camera_view(id)?;
                self.edit(
                    "Delete saved camera view",
                    &[Command::DeleteCameraView { id }],
                )?;
                self.cancel_camera_view_name();
                Ok(())
            }
            V::Rebind(id) => {
                let view = self.saved_camera_view(id)?;
                let preview = self.camera_preview()?;
                if !preview.interactive {
                    return Err("Bind a saved view only from one 3D replay view.".into());
                }
                let source = self
                    .project()?
                    .sources
                    .iter()
                    .find(|source| source.id == preview.source)
                    .ok_or("The displayed take is no longer in this project.")?;
                let view = view.rebound(source).map_err(|error| error.to_string())?;
                self.edit(
                    "Bind saved view to current take",
                    &[Command::ReplaceCameraView { view }],
                )?;
                self.cancel_camera_view_name();
                Ok(())
            }
            V::Page(direction) => {
                let total = self.project()?.camera_views.len();
                Arc::make_mut(&mut self.view)
                    .camera_workspace
                    .saved_views
                    .step(total, direction);
                Ok(())
            }
        }
    }

    /// A missing stable identity is an error rather than another row being edited after list changes.
    fn saved_camera_view(&self, id: Id) -> Result<SavedCameraView, String> {
        self.project()?
            .camera_views
            .iter()
            .find(|view| view.id == id)
            .cloned()
            .ok_or_else(|| "This saved view is no longer in the project.".into())
    }

    /// Captures the displayed pose once; later camera movement cannot change the view awaiting a name.
    pub(in crate::menu) fn begin_camera_view_name(&mut self, id: Option<Id>) -> Result<(), String> {
        let revision = self
            .editor
            .as_ref()
            .ok_or("Open a project first.")?
            .revision();
        if self.camera_view_edit.as_ref().is_some_and(|edit| {
            edit.document == self.document
                && edit.revision == revision
                && edit.original.as_ref().map(|view| view.id) == id
        }) {
            return Ok(());
        }
        let original = id.map(|id| self.saved_camera_view(id)).transpose()?;
        let view = if let Some(view) = &original {
            view.clone()
        } else {
            let preview = self.camera_preview()?;
            if !preview.interactive {
                return Err("Save a view from one unframed 3D replay view.".into());
            }
            let project = self.project()?;
            let source = project
                .sources
                .iter()
                .find(|source| source.id == preview.source)
                .ok_or("The displayed take is no longer in this project.")?;
            let pose = self
                .view
                .camera_workspace
                .view
                .map_or(preview.render_pose, |view| view.pose);
            SavedCameraView::new(
                project.next_id().map_err(|error| error.to_string())?,
                format!("View {}", project.camera_views.len() + 1),
                source,
                pose,
            )
            .map_err(|error| error.to_string())?
        };
        self.transport.pause();
        self.release_camera();
        self.camera_view_text.set_text(&view.name);
        self.camera_view_edit = Some(CameraViewEdit {
            document: self.document,
            revision,
            original,
            view,
        });
        self.publish_camera_view_name();
        Ok(())
    }

    /// Only presentation text changes while typing; committing remains one explicit undo step.
    pub(in crate::menu) fn publish_camera_view_name(&mut self) {
        if let Some(edit) = &self.camera_view_edit {
            Arc::make_mut(&mut self.view)
                .camera_workspace
                .saved_views
                .draft = Some(CameraViewNameDraft {
                id: edit.original.as_ref().map(|view| view.id),
                text: Arc::from(self.camera_view_text.as_str()),
            });
        }
    }

    /// A stale draft never overwrites another source, renamed view or newly allocated project identity.
    fn save_camera_view_name(&mut self) -> Result<(), String> {
        let edit = self
            .camera_view_edit
            .as_ref()
            .ok_or("Choose Save view or Rename first.")?;
        if edit.document != self.document
            || self
                .editor
                .as_ref()
                .is_none_or(|editor| editor.revision() != edit.revision)
        {
            return Err(
                "The project changed while this name was open. Cancel and save the view again."
                    .into(),
            );
        }
        let mut view = edit.view.clone();
        view.name = self.camera_view_text.as_str().trim().to_owned();
        let id = view.id;
        let rename = edit.original.is_some();
        let command = if rename {
            Command::ReplaceCameraView { view }
        } else {
            Command::AddCameraView { view }
        };
        self.edit(
            if rename {
                "Rename saved camera view"
            } else {
                "Save camera view"
            },
            &[command],
        )?;
        self.cancel_camera_view_name();
        let index = self
            .project()?
            .camera_views
            .iter()
            .position(|view| view.id == id)
            .unwrap_or(0);
        let saved = &mut Arc::make_mut(&mut self.view).camera_workspace.saved_views;
        saved.selected = Some(id);
        saved.first = index;
        Ok(())
    }

    /// Cancelling or entering another text field discards naming text without editing the document.
    pub(in crate::menu) fn cancel_camera_view_name(&mut self) {
        if self.camera_view_edit.is_none() && self.view.camera_workspace.saved_views.draft.is_none()
        {
            return;
        }
        self.camera_view_edit = None;
        self.camera_view_text.set_text("");
        Arc::make_mut(&mut self.view)
            .camera_workspace
            .saved_views
            .draft = None;
    }

    /// Deleted views and shorter undo revisions invalidate only selection, never silently recall another view.
    pub(in crate::menu::studio) fn reconcile_saved_camera_views(&mut self) {
        let project = self.editor.as_ref().map(|editor| editor.project());
        let saved = &self.view.camera_workspace.saved_views;
        let valid = saved.selected.is_none_or(|id| {
            project.is_some_and(|project| project.camera_views.iter().any(|view| view.id == id))
        });
        let count = project.map_or(0, |project| project.camera_views.len());
        let first = saved.range(count).start;
        if !valid || first != saved.first {
            let saved = &mut Arc::make_mut(&mut self.view).camera_workspace.saved_views;
            if !valid {
                saved.selected = None;
            }
            saved.first = first;
        }
    }

    /// Native focus follows the same bounded saved-view page as the painted list.
    pub(super) fn saved_camera_view_focus_actions(&self) -> Vec<Action> {
        if !self.toolbox_active(launcher::menu::studio::Toolbox::Cameras)
            || self.view.camera_workspace.tools != launcher::menu::studio::CameraTools::Views
        {
            return Vec::new();
        }
        let Ok(project) = self.project() else {
            return Vec::new();
        };
        let saved = &self.view.camera_workspace.saved_views;
        let mut actions = Vec::new();
        let preview = self
            .view
            .preview
            .camera
            .as_ref()
            .filter(|preview| self.view.preview.ready && preview.interactive);
        if preview.is_some() && saved.draft.is_none() {
            actions.push(Action::CameraViews(V::New));
        }
        for view in &project.camera_views[saved.range(project.camera_views.len())] {
            actions.push(Action::CameraViews(V::Select(view.id)));
        }
        if let Some(view) = saved
            .selected
            .and_then(|id| project.camera_views.iter().find(|view| view.id == id))
        {
            let available =
                preview.is_some_and(|preview| view.recall(project, preview.source).is_ok());
            if available {
                actions.push(Action::CameraViews(V::Recall(view.id)));
            }
            if preview.is_some() && !available {
                actions.push(Action::CameraViews(V::Rebind(view.id)));
            }
            actions.extend([V::Rename(view.id), V::Delete(view.id)].map(Action::CameraViews));
        }
        if let Some(draft) = &saved.draft {
            actions.push(Action::CameraViews(draft.id.map_or(V::New, V::Rename)));
            actions.extend([V::Save, V::Cancel].map(Action::CameraViews));
        }
        if saved.range(project.camera_views.len()).start > 0 {
            actions.push(Action::CameraViews(V::Page(-1)));
        }
        if saved.range(project.camera_views.len()).end < project.camera_views.len() {
            actions.push(Action::CameraViews(V::Page(1)));
        }
        actions
    }
}

#[cfg(test)]
mod tests;
