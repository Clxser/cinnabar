//! Graph gestures retain a frozen document and publish cancellable drafts before one undoable commit.

use super::*;
use launcher::menu::studio::graph::{GraphAction, GraphPublication};
use recording_studio::{
    camera::CameraPose,
    graph::{self, GraphChannel, GraphControl, GraphCurve, GraphKey},
};

mod pointer;
#[cfg(test)]
mod tests;
mod values;

#[derive(Default)]
pub(super) struct State {
    pub(super) project: Option<Arc<Project>>,
    gesture: Option<pointer::Gesture>,
    base: Option<CameraPose>,
    numeric: Option<values::NumericEdit>,
}

impl Studio {
    /// Stale painted controls cannot alter transport or framing through a replacement screen.
    pub(super) fn graph_context_available(&self) -> bool {
        self.editor.is_some()
            && !self.view.show_library
            && !self.view.loading
            && self.view.discard.is_none()
            && !self.view.workflow.show_settings
            && !self.view.workflow.recovery.visible
            && !self.view.workflow.recovery.busy
    }
    /// One publication owns all painted hit coordinates and cannot outlive its selected document.
    pub(super) fn graph_publication_current(&self) -> bool {
        self.view
            .camera_workspace
            .graph
            .publication
            .as_ref()
            .is_some_and(|p| {
                self.view.selected == Some(p.curve.item)
                    && self.view.camera_workspace.graph.channel == Some(p.curve.channel)
                    && self
                        .editor
                        .as_ref()
                        .is_some_and(|editor| Arc::ptr_eq(editor.project(), &p.project))
            })
    }
    /// A graph value drag exclusively owns pointer navigation until its original release or cancellation.
    pub(in crate::menu) fn graph_drag_active(&self) -> bool {
        self.graph.gesture.is_some()
    }
    /// The graph uses pointer-anchored horizontal zoom and independent vertical value framing.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::menu) fn graph_wheel(
        &mut self,
        point: ui::UiPoint,
        hovered: Option<launcher::menu::MenuAction>,
        horizontal: f32,
        vertical: f32,
        pixels: bool,
        shift: bool,
        zoom: bool,
    ) -> bool {
        let Some(launcher::menu::MenuAction::Studio(Action::Graph(
            GraphAction::Scrub { bounds } | GraphAction::Drag { bounds, .. },
        ))) = hovered
        else {
            return false;
        };
        if !self.graph_context_available()
            || !self.view.camera_workspace.graph.open
            || !self.graph_publication_current()
        {
            return true;
        }
        if !horizontal.is_finite() || !vertical.is_finite() || self.graph.gesture.is_some() {
            return true;
        }
        let fraction = (f64::from(point.x()) - f64::from(bounds[0]))
            / f64::from((bounds[2] - bounds[0]).max(1));
        if zoom {
            self.zoom_timeline(
                f64::from(vertical) / if pixels { 160.0 } else { 4.0 },
                fraction,
            );
        } else if horizontal != 0.0 || shift {
            self.pan_timeline(
                -f64::from(if horizontal != 0.0 {
                    horizontal
                } else {
                    vertical
                }) / if pixels {
                    f64::from((bounds[2] - bounds[0]).max(1))
                } else {
                    12.0
                },
            );
        } else if let Some(publication) = &self.view.camera_workspace.graph.publication {
            let [low, high] = self
                .view
                .camera_workspace
                .graph
                .range
                .unwrap_or(publication.curve.range);
            let anchor = ((f64::from(point.y()) - f64::from(bounds[1]))
                / f64::from((bounds[3] - bounds[1]).max(1)))
            .clamp(0.0, 1.0);
            let factor = 2.0_f64
                .powf((-f64::from(vertical) / if pixels { 320.0 } else { 8.0 }).clamp(-12.0, 12.0));
            Arc::make_mut(&mut self.view).camera_workspace.graph.range =
                Some(transform_range([low, high], 1.0 - anchor, factor, 0.0));
        }
        self.publish_editor();
        true
    }

    /// Keyboard focus exposes the same editing controls when painted pointer targets are unavailable.
    pub(super) fn graph_focus_actions(&self) -> Vec<Action> {
        let mut actions = vec![Action::Graph(GraphAction::Toggle)];
        if !self.view.camera_workspace.graph.open {
            return actions;
        }
        if let Some(publication) = &self.view.camera_workspace.graph.publication {
            actions.extend(
                graph::channels(&publication.project, publication.curve.item)
                    .into_iter()
                    .map(|channel| Action::Graph(GraphAction::Channel(channel))),
            );
            actions.extend(
                [
                    GraphAction::AddKey,
                    GraphAction::DeleteKey,
                    GraphAction::CycleEasing,
                    GraphAction::Fit,
                    GraphAction::Reset,
                    GraphAction::ZoomValue(1),
                    GraphAction::ZoomValue(-1),
                    GraphAction::PanValue(1),
                    GraphAction::PanValue(-1),
                ]
                .into_iter()
                .map(Action::Graph),
            );
            if self.view.camera_workspace.graph.selected.is_some() {
                actions.extend(
                    [
                        GraphAction::Edit(launcher::menu::studio::graph::GraphField::Time),
                        GraphAction::Edit(launcher::menu::studio::graph::GraphField::Value),
                    ]
                    .map(Action::Graph),
                );
                if self.view.camera_workspace.graph.draft.is_some() {
                    actions.extend([GraphAction::Apply, GraphAction::Cancel].map(Action::Graph));
                }
                actions.extend(
                    [
                        GraphAction::Nudge { time: -1, value: 0 },
                        GraphAction::Nudge { time: 1, value: 0 },
                        GraphAction::Nudge { time: 0, value: -1 },
                        GraphAction::Nudge { time: 0, value: 1 },
                    ]
                    .into_iter()
                    .map(Action::Graph),
                );
            }
        }
        actions
    }

    /// Rebuilds graph data only for immutable document, selection or channel changes.
    pub(super) fn reconcile_graph(&mut self) {
        if !self.view.camera_workspace.graph.open {
            self.cancel_graph_drag();
            return;
        }
        if self.editor.is_none() {
            return;
        }
        self.reconcile_graph_value();
        if self
            .graph
            .gesture
            .as_ref()
            .is_some_and(|g| !g.current(self))
        {
            self.cancel_graph_drag();
        }
        let project = self
            .graph
            .project
            .as_ref()
            .unwrap_or_else(|| self.editor.as_ref().unwrap().project());
        let Some(item) = self.view.selected else {
            Arc::make_mut(&mut self.view)
                .camera_workspace
                .graph
                .publication = None;
            return;
        };
        let channels = graph::channels(project, item);
        let state = &self.view.camera_workspace.graph;
        let channel = state
            .channel
            .filter(|c| channels.contains(c))
            .or_else(|| channels.first().copied());
        let Some(channel) = channel else {
            Arc::make_mut(&mut self.view)
                .camera_workspace
                .graph
                .publication = None;
            return;
        };
        if state.publication.as_ref().is_some_and(|p| {
            Arc::ptr_eq(&p.project, project) && p.curve.item == item && p.curve.channel == channel
        }) {
            return;
        }
        let base = *self.graph.base.get_or_insert_with(|| {
            self.view
                .preview
                .camera
                .as_ref()
                .map_or_else(CameraPose::default, |p| p.pose)
        });
        let publication = GraphCurve::new_reusing(
            Arc::clone(project),
            item,
            channel,
            base,
            state.publication.as_ref().map(|p| p.curve.as_ref()),
        )
        .ok()
        .map(|curve| GraphPublication {
            project: Arc::clone(project),
            curve: Arc::new(curve),
        });
        let workspace = &mut Arc::make_mut(&mut self.view).camera_workspace.graph;
        if workspace.channel != Some(channel)
            || workspace
                .publication
                .as_ref()
                .is_some_and(|p| p.curve.item != item)
        {
            workspace.range = None;
            workspace.selected = None;
        }
        workspace.channel = Some(channel);
        workspace.publication = publication;
        workspace.selected = workspace.selected.filter(|key| {
            workspace.publication.as_ref().is_some_and(|p| {
                p.curve.item == key.item
                    && p.curve.channel == key.channel
                    && p.curve.points.iter().any(|point| point.at == key.at)
            })
        });
    }

    /// Paints reuse bounded samples; a stationary viewer never reintegrates rail coordinates.
    pub(super) fn reconcile_graph_plot(&mut self) {
        let state = &self.view.camera_workspace.graph;
        let Some(publication) = state.publication.as_ref().filter(|_| state.open) else {
            return;
        };
        let window = [self.view.visible_start, self.view.visible_duration];
        if state.plot.as_ref().is_some_and(|plot| {
            Arc::ptr_eq(&plot.curve, &publication.curve) && plot.window == window
        }) {
            return;
        }
        let first = window[0].max(publication.curve.span.start);
        let last =
            MediaTime(window[0].0.saturating_add(window[1].0)).min(publication.curve.span.end);
        let points = if first <= last {
            (0..=512_u128)
                .map(|i| {
                    let at = MediaTime(first.0 + ((u128::from(last.0 - first.0) * i) / 512) as u64);
                    [at.0 as f64, publication.curve.sample(at)]
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let plot = launcher::menu::studio::graph::GraphPlot {
            curve: Arc::clone(&publication.curve),
            window,
            points: points.into(),
        };
        Arc::make_mut(&mut self.view).camera_workspace.graph.plot = Some(plot);
    }

    /// Shared saved-owner graph mutations use normal transactional edit and lock admission.
    pub(super) fn graph_action(&mut self, action: GraphAction) -> Result<(), String> {
        if action != GraphAction::Cancel && !self.graph_context_available() {
            return Err("Close the current overlay before editing curves.".into());
        }
        match action {
            GraphAction::Edit(field) => return self.begin_graph_value(field),
            GraphAction::Apply => return self.apply_graph_value(),
            GraphAction::Cancel => {
                self.cancel_graph_value();
                return Ok(());
            }
            _ => self.cancel_graph_value(),
        }
        if matches!(action, GraphAction::Toggle) {
            self.cancel_graph_drag();
            self.release_camera();
            let graph = &mut Arc::make_mut(&mut self.view).camera_workspace.graph;
            graph.open = !graph.open;
            self.graph.base = None;
            return Ok(());
        }
        if let GraphAction::Channel(channel) = action {
            self.cancel_graph_drag();
            let graph = &mut Arc::make_mut(&mut self.view).camera_workspace.graph;
            graph.channel = Some(channel);
            graph.selected = None;
            graph.range = None;
            graph.publication = None;
            return Ok(());
        }
        let publication = self
            .view
            .camera_workspace
            .graph
            .publication
            .clone()
            .ok_or("Select a camera, effect or clip with speed keys to edit its curves.")?;
        let curve = &publication.curve;
        let key = self.view.camera_workspace.graph.selected;
        let base = self.graph.base.unwrap_or_default();
        match action {
            GraphAction::Select(key) => {
                if key.item != curve.item
                    || key.channel != curve.channel
                    || !curve.points.iter().any(|point| point.at == key.at)
                {
                    return Err("This curve key no longer exists.".into());
                }
                Arc::make_mut(&mut self.view)
                    .camera_workspace
                    .graph
                    .selected = Some(key);
                self.transport.pause();
                self.transport.seek(key.at, self.project()?.duration());
            }
            GraphAction::Fit => {
                let view = Arc::make_mut(&mut self.view);
                view.camera_workspace.graph.range = Some(curve.range);
                view.visible_start = curve.span.start;
                view.visible_duration = curve.span.duration();
            }
            GraphAction::ZoomValue(direction) | GraphAction::PanValue(direction) => {
                let [low, high] = self
                    .view
                    .camera_workspace
                    .graph
                    .range
                    .unwrap_or(curve.range);
                let range = if matches!(action, GraphAction::ZoomValue(_)) {
                    transform_range(
                        [low, high],
                        0.5,
                        2.0_f64.powf(-f64::from(direction) * 0.5),
                        0.0,
                    )
                } else {
                    transform_range([low, high], 0.5, 1.0, f64::from(direction) * 0.125)
                };
                Arc::make_mut(&mut self.view).camera_workspace.graph.range = Some(range);
            }
            GraphAction::AddKey
            | GraphAction::DeleteKey
            | GraphAction::CycleEasing
            | GraphAction::Reset
            | GraphAction::Nudge { .. } => {
                let project = self.project()?;
                let track = recording_studio::tracks::item_track(project, curve.item)
                    .map_err(|e| e.to_string())?;
                if self.view.workspace.timeline.locked.contains(&track) {
                    return Err("Unlock the selected track before editing its curve.".into());
                }
                let command = match action {
                    GraphAction::AddKey => graph::set_key_on(
                        curve,
                        project,
                        None,
                        self.transport
                            .position()
                            .clamp(curve.span.start, curve.span.end),
                        curve.sample(self.transport.position()),
                    ),
                    GraphAction::DeleteKey => {
                        graph::delete_key(project, key.ok_or("Select a curve key first.")?, base)
                    }
                    GraphAction::CycleEasing => {
                        let key = key.ok_or("Select a curve key first.")?;
                        let point = curve
                            .points
                            .iter()
                            .find(|p| p.at == key.at)
                            .ok_or("This curve key no longer exists.")?;
                        graph::set_easing_on(curve, project, key, point.easing.next())
                    }
                    GraphAction::Reset => graph::reset(project, curve.item, curve.channel),
                    GraphAction::Nudge { time, value } => {
                        let key = key.ok_or("Select a curve key first.")?;
                        let point = curve
                            .points
                            .iter()
                            .find(|p| p.at == key.at)
                            .ok_or("This curve key no longer exists.")?;
                        let fps = project.export.fps;
                        let at = if time == 0 {
                            point.at
                        } else {
                            let index = fps
                                .frame_at(fps.snap(point.at).map_err(|e| e.to_string())?)
                                .map_err(|e| e.to_string())?;
                            let next = (i128::from(index) + i128::from(time)).max(0) as u64;
                            fps.frame_time(next)
                                .map_err(|e| e.to_string())?
                                .clamp(curve.span.start, curve.span.end)
                        };
                        let [low, high] = self
                            .view
                            .camera_workspace
                            .graph
                            .range
                            .unwrap_or(curve.range);
                        graph::set_key_on(
                            curve,
                            project,
                            Some(key.at),
                            at,
                            if value == 0 {
                                point.value
                            } else {
                                nudge_value(point.value, [low, high], f64::from(value) * 0.01)
                            },
                        )
                    }
                    _ => unreachable!(),
                }
                .map_err(|e| e.to_string())?;
                let index = key.and_then(|key| curve.points.iter().position(|p| p.at == key.at));
                self.cancel_graph_drag();
                self.edit("Edit curve", &[command])?;
                Arc::make_mut(&mut self.view).camera_workspace.view = None;
                self.release_camera();
                self.reconcile_graph();
                let next = if matches!(action, GraphAction::Nudge { .. } | GraphAction::CycleEasing)
                {
                    index
                        .and_then(|i| {
                            self.view
                                .camera_workspace
                                .graph
                                .publication
                                .as_ref()?
                                .curve
                                .points
                                .get(i)
                        })
                        .map(|point| GraphKey {
                            at: point.at,
                            item: curve.item,
                            channel: curve.channel,
                        })
                } else {
                    None
                };
                Arc::make_mut(&mut self.view)
                    .camera_workspace
                    .graph
                    .selected = next;
                if let Some(key) = next {
                    self.transport.pause();
                    self.transport.seek(key.at, self.project()?.duration());
                }
            }
            GraphAction::Drag { .. }
            | GraphAction::Scrub { .. }
            | GraphAction::Toggle
            | GraphAction::Channel(_) => {}
            GraphAction::Edit(_) | GraphAction::Apply | GraphAction::Cancel => unreachable!(),
        }
        Ok(())
    }

    /// Cancellation restores the saved document and drops the transient preview owner.
    pub(super) fn cancel_graph_drag(&mut self) {
        if let Some(gesture) = self.graph.gesture.take() {
            Arc::make_mut(&mut self.view).camera_workspace.view = gesture.inspection;
        }
        self.graph.project = None;
    }
}

/// Scaled arithmetic keeps custom value framing finite even for opposite extreme scene coordinates.
fn transform_range(range: [f64; 2], anchor: f64, factor: f64, pan: f64) -> [f64; 2] {
    let scale = range[0].abs().max(range[1].abs()).max(1.0);
    let [a, b] = range.map(|v| v / scale);
    let at = a * (1.0 - anchor) + b * anchor;
    let delta = (b - a) * pan;
    let next = [
        at + (a - at) * factor + delta,
        at + (b - at) * factor + delta,
    ]
    .map(|v| (v * scale).clamp(-f64::MAX, f64::MAX));
    if next[0] < next[1] && next.iter().all(|v| v.is_finite()) {
        next
    } else {
        range
    }
}

/// Keyboard nudges retain off-screen values and use scaled arithmetic for large finite coordinates.
fn nudge_value(value: f64, range: [f64; 2], amount: f64) -> f64 {
    let scale = range[0].abs().max(range[1].abs()).max(value.abs()).max(1.0);
    ((value / scale + (range[1] / scale - range[0] / scale) * amount) * scale)
        .clamp(-f64::MAX, f64::MAX)
}
