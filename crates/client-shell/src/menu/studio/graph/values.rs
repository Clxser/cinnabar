//! Exact key fields retain their original document and curve until one explicit commit.

use super::*;
use launcher::menu::studio::graph::GraphField;

pub(super) struct NumericEdit {
    project: Arc<Project>,
    curve: Arc<GraphCurve>,
    key: GraphKey,
    field: GraphField,
    document: u64,
    revision: u64,
}

impl Studio {
    /// Numeric values use seconds and displayed units; speed values are multipliers.
    pub(in crate::menu) fn begin_graph_value(&mut self, field: GraphField) -> Result<(), String> {
        if !self.graph_context_available() || !self.view.camera_workspace.graph.open {
            return Err("Close the current overlay before editing curves.".into());
        }
        self.cancel_graph_drag();
        let publication = self
            .view
            .camera_workspace
            .graph
            .publication
            .clone()
            .ok_or("Select a curve first.")?;
        let key = self
            .view
            .camera_workspace
            .graph
            .selected
            .ok_or("Select a curve key first.")?;
        let point = publication
            .curve
            .points
            .iter()
            .find(|p| p.at == key.at)
            .ok_or("This curve key no longer exists.")?;
        let editor = self.editor.as_ref().ok_or("Open a project first.")?;
        let track = recording_studio::tracks::item_track(editor.project(), key.item)
            .map_err(|e| e.to_string())?;
        if !Arc::ptr_eq(editor.project(), &publication.project)
            || key.item != publication.curve.item
            || key.channel != publication.curve.channel
            || self.view.workspace.timeline.locked.contains(&track)
        {
            return Err("This curve changed or its track is locked. Select it again.".into());
        }
        if self.graph.numeric.as_ref().is_some_and(|edit| {
            edit.field == field && edit.key == key && Arc::ptr_eq(&edit.project, editor.project())
        }) {
            return Ok(());
        }
        let value = match field {
            GraphField::Time => point.at.seconds(),
            GraphField::Value => {
                if key.channel == GraphChannel::Speed {
                    point.value.exp2()
                } else {
                    point.value
                }
            }
        };
        let text = value.to_string();
        self.graph_text.set_text(
            &if text.len() > recording_studio::camera::MAX_CAMERA_VALUE_BYTES {
                format!("{value:e}")
            } else {
                text
            },
        );
        self.graph.numeric = Some(NumericEdit {
            project: Arc::clone(editor.project()),
            curve: publication.curve,
            key,
            field,
            document: self.document,
            revision: editor.revision(),
        });
        self.transport.pause();
        self.release_camera();
        self.publish_graph_value();
        Ok(())
    }

    /// Typing updates presentation text without scheduling replay work or creating history.
    pub(in crate::menu) fn publish_graph_value(&mut self) {
        if let Some(edit) = &self.graph.numeric {
            Arc::make_mut(&mut self.view).camera_workspace.graph.draft =
                Some((edit.field, Arc::from(self.graph_text.as_str())));
        }
    }

    /// Replaced documents, changed selections and locked tracks cannot receive a stale numeric edit.
    pub(super) fn reconcile_graph_value(&mut self) {
        let current = self.graph.numeric.as_ref().is_none_or(|edit| {
            self.document == edit.document
                && self.editor.as_ref().is_some_and(|editor| {
                    editor.revision() == edit.revision
                        && Arc::ptr_eq(editor.project(), &edit.project)
                })
                && self.view.camera_workspace.graph.open
                && self.view.camera_workspace.graph.channel == Some(edit.key.channel)
                && self.view.camera_workspace.graph.selected == Some(edit.key)
                && self.view.selected == Some(edit.key.item)
                && !self.view.show_library
                && !self.view.loading
                && self.view.discard.is_none()
                && !self.view.workflow.show_settings
                && !self.view.workflow.recovery.visible
                && recording_studio::tracks::item_track(&edit.project, edit.key.item)
                    .is_ok_and(|track| !self.view.workspace.timeline.locked.contains(&track))
        });
        if !current {
            self.cancel_graph_value();
        }
    }

    /// Applies one admitted key replacement, preserving its exact source identity and undo ownership.
    pub(super) fn apply_graph_value(&mut self) -> Result<(), String> {
        self.reconcile_graph_value();
        let edit = self
            .graph
            .numeric
            .as_ref()
            .ok_or("This curve changed. Reopen the value field.")?;
        let value = self
            .graph_text
            .as_str()
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or("Enter a finite number using a decimal point.")?;
        let point = edit
            .curve
            .points
            .iter()
            .find(|p| p.at == edit.key.at)
            .ok_or("This curve key no longer exists.")?;
        let (at, value) = match edit.field {
            GraphField::Time => (
                MediaTime::from_seconds(value).map_err(|e| e.to_string())?,
                point.value,
            ),
            GraphField::Value => (
                point.at,
                if edit.key.channel == GraphChannel::Speed {
                    if value <= 0.0 {
                        return Err("Speed must be greater than zero.".into());
                    }
                    value.log2()
                } else {
                    value
                },
            ),
        };
        let index = edit
            .curve
            .points
            .iter()
            .position(|p| p.at == edit.key.at)
            .unwrap();
        let command = graph::set_key_on(&edit.curve, &edit.project, Some(edit.key.at), at, value)
            .map_err(|e| e.to_string())?;
        self.edit("Set curve key", &[command])?;
        Arc::make_mut(&mut self.view).camera_workspace.view = None;
        self.release_camera();
        self.cancel_graph_value();
        self.reconcile_graph();
        let next = self
            .view
            .camera_workspace
            .graph
            .publication
            .as_ref()
            .and_then(|publication| {
                publication.curve.points.get(index).map(|point| GraphKey {
                    at: point.at,
                    item: publication.curve.item,
                    channel: publication.curve.channel,
                })
            });
        Arc::make_mut(&mut self.view)
            .camera_workspace
            .graph
            .selected = next;
        if let Some(key) = next {
            self.transport.seek(key.at, self.project()?.duration());
        }
        Ok(())
    }

    /// Cancelling a numeric draft leaves the saved project and current preview unchanged.
    pub(in crate::menu) fn cancel_graph_value(&mut self) {
        self.graph.numeric = None;
        self.graph_text.set_text("");
        Arc::make_mut(&mut self.view).camera_workspace.graph.draft = None;
    }
}
