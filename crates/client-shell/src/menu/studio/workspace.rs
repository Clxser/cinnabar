//! Panel navigation and density remain independent of the edited document and its source clock.

use super::*;
use launcher::menu::studio::{
    EditorPanel, InspectorSection, SettingsPage, TimelineSize, Toolbox, ViewportTool,
    WorkspaceAction, WorkspaceLayout,
};

impl Studio {
    /// New selections reveal their properties once; hiding the panel stays respected until selection changes.
    pub(super) fn reconcile_inspector_selection(&mut self) {
        let selected = self.view.selected;
        if self.view.workspace.property_selection == selected {
            return;
        }
        let editable = self.editor.as_ref().is_some_and(|editor| {
            let project = editor.project();
            project.clips.iter().any(|item| Some(item.id) == selected)
                || project.shots.iter().any(|item| Some(item.id) == selected)
                || project.effects.iter().any(|item| Some(item.id) == selected)
                || project.audio.iter().any(|item| Some(item.id) == selected)
                || project.titles.iter().any(|item| Some(item.id) == selected)
        });
        let workspace = &mut Arc::make_mut(&mut self.view).workspace;
        workspace.property_selection = selected;
        if editable {
            workspace.inspector_hidden = false;
            workspace.panel = EditorPanel::Inspector;
        }
    }

    /// Enumerates workspace tools from their registry before painted focus targets are available.
    pub(super) fn workspace_focus_actions(&self) -> Vec<Action> {
        let mut actions: Vec<_> = Toolbox::ALL
            .iter()
            .copied()
            .map(|tool| WorkspaceAction::Toolbox(tool))
            .chain(
                InspectorSection::ALL
                    .iter()
                    .copied()
                    .map(WorkspaceAction::Inspector),
            )
            .collect();
        actions.extend([
            WorkspaceAction::ToggleInspector,
            WorkspaceAction::PreviewOnly,
            WorkspaceAction::Density,
            WorkspaceAction::TimelineSize,
            WorkspaceAction::CycleLayout,
            WorkspaceAction::TogglePreviewFill,
            WorkspaceAction::CyclePreviewQuality,
            WorkspaceAction::DismissStatus,
        ]);
        actions.extend(
            SettingsPage::ALL
                .iter()
                .copied()
                .map(WorkspaceAction::SettingsPage),
        );
        if self
            .view
            .preview
            .camera
            .as_ref()
            .is_some_and(|camera| camera.interactive)
        {
            actions.extend(
                ViewportTool::ALL
                    .iter()
                    .copied()
                    .map(WorkspaceAction::ViewportTool),
            );
        }
        actions.into_iter().map(Action::Workspace).collect()
    }

    /// Releases hidden drafts when switching property groups and leaves project undo untouched.
    pub(super) fn workspace_action(&mut self, action: WorkspaceAction) {
        if action == WorkspaceAction::DismissStatus {
            let view = Arc::make_mut(&mut self.view);
            view.error = None;
            view.notice = None;
            return;
        }
        self.release_camera();
        self.cancel_camera_value();
        self.cancel_clip_time();
        self.cancel_title_text();
        if matches!(action, WorkspaceAction::SettingsPage(_)) {
            self.transport.pause();
        }
        let view = Arc::make_mut(&mut self.view);
        if matches!(action, WorkspaceAction::SettingsPage(_)) {
            view.workflow.show_settings = true;
        }
        let workspace = &mut view.workspace;
        match action {
            WorkspaceAction::Toolbox(tool) => {
                let hide = workspace.toolbox == tool
                    && workspace.panel == EditorPanel::Media
                    && !workspace.media_hidden
                    && !workspace.preview_only;
                workspace.toolbox = tool;
                workspace.media_hidden = hide;
                workspace.preview_only = false;
                workspace.panel = if hide {
                    EditorPanel::Preview
                } else {
                    EditorPanel::Media
                };
            }
            WorkspaceAction::Inspector(section) => {
                workspace.inspector = section;
                workspace.inspector_hidden = false;
                workspace.preview_only = false;
                workspace.panel = EditorPanel::Inspector;
            }
            WorkspaceAction::ToggleInspector => {
                workspace.inspector_hidden = !workspace.inspector_hidden;
                workspace.preview_only = false;
                workspace.panel = if workspace.inspector_hidden {
                    EditorPanel::Preview
                } else {
                    EditorPanel::Inspector
                };
            }
            WorkspaceAction::PreviewOnly => workspace.preview_only = !workspace.preview_only,
            WorkspaceAction::Density => workspace.density = workspace.density.next(),
            WorkspaceAction::TimelineSize => {
                workspace.timeline_size = workspace.timeline_size.next()
            }
            WorkspaceAction::CycleLayout => {
                workspace.layout = workspace.layout.next();
                workspace.preview_only = false;
                workspace.panel = EditorPanel::Preview;
                match workspace.layout {
                    WorkspaceLayout::Edit => {
                        workspace.media_hidden = true;
                        workspace.inspector_hidden = view.selected.is_none();
                        workspace.timeline_size = TimelineSize::Small;
                    }
                    WorkspaceLayout::Review => {
                        workspace.media_hidden = true;
                        workspace.inspector_hidden = true;
                        workspace.timeline_size = TimelineSize::Small;
                    }
                    WorkspaceLayout::Audio => {
                        workspace.media_hidden = false;
                        workspace.toolbox = Toolbox::Audio;
                        workspace.inspector_hidden = true;
                        workspace.panel = EditorPanel::Media;
                        workspace.timeline_size = TimelineSize::Large;
                    }
                }
            }
            WorkspaceAction::ViewportTool(tool) => workspace.viewport_tool = tool,
            WorkspaceAction::TogglePreviewFill => workspace.preview_fill = !workspace.preview_fill,
            WorkspaceAction::CyclePreviewQuality => {
                workspace.preview_quality = workspace.preview_quality.next()
            }
            WorkspaceAction::SettingsPage(page) => workspace.settings_page = page,
            WorkspaceAction::DismissStatus => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties_open_for_a_new_item_and_respect_an_explicit_hide_until_selection_changes() {
        let mut studio = Studio::new(PathBuf::new());
        studio.install(Editor::new(Project::new("Editing")).unwrap(), None);
        assert!(studio.view.workspace.media_hidden);
        assert!(studio.view.workspace.inspector_hidden);
        studio.activate(Action::AddTitle);
        let first = studio.view.selected;
        assert!(first.is_some());
        assert!(!studio.view.workspace.inspector_hidden);
        studio.activate(Action::Workspace(WorkspaceAction::ToggleInspector));
        assert!(studio.view.workspace.inspector_hidden);
        studio.publish_editor();
        assert!(studio.view.workspace.inspector_hidden);
        studio.activate(Action::AddTitle);
        assert_ne!(studio.view.selected, first);
        assert!(!studio.view.workspace.inspector_hidden);
    }
}
