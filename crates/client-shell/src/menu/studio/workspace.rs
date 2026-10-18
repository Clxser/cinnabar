//! Panel navigation and density remain independent of the edited document and its source clock.

use super::*;
use launcher::menu::studio::{
    EditorPanel, InspectorSection, SettingsPage, Toolbox, ViewportTool, WorkspaceAction,
};

impl Studio {
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
