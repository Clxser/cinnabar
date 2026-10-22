//! Frozen graph geometry prevents a moving preview or autosizing range from changing the drag's meaning.

use super::*;

pub(super) struct Gesture {
    pub(super) project: Arc<Project>,
    pub(super) inspection: Option<recording_studio::camera::CameraView>,
    curve: Arc<GraphCurve>,
    key: GraphKey,
    control: GraphControl,
    bounds: [i32; 4],
    down: [f32; 2],
    origin: [f64; 2],
    last: [f32; 2],
    window: [MediaTime; 2],
    range: [f64; 2],
    pending: Option<Command>,
    result_key: GraphKey,
    epoch: [u64; 3],
}

impl Gesture {
    /// A modal, seek, selection, lock or document change invalidates the complete captured gesture.
    pub(super) fn current(&self, studio: &Studio) -> bool {
        studio.document == self.epoch[0]
            && studio.editor.as_ref().is_some_and(|editor| {
                editor.revision() == self.epoch[1] && Arc::ptr_eq(editor.project(), &self.project)
            })
            && studio.transport.epoch() == self.epoch[2]
            && !studio.transport.playing()
            && studio.view.camera_workspace.graph.open
            && studio.view.camera_workspace.graph.channel == Some(self.key.channel)
            && studio.view.selected == Some(self.key.item)
            && !studio.view.show_library
            && !studio.view.loading
            && studio.view.discard.is_none()
            && !studio.view.workflow.show_settings
            && !studio.view.workflow.recovery.visible
            && !studio.view.workflow.recovery.busy
            && recording_studio::tracks::item_track(&self.project, self.key.item)
                .is_ok_and(|track| !studio.view.workspace.timeline.locked.contains(&track))
    }
}

impl Studio {
    /// A graph drag previews complete admitted metadata and commits one undo step on its original release.
    pub(in crate::menu::studio) fn graph_pointer(
        &mut self,
        hovered: Option<launcher::menu::MenuAction>,
        point: Option<[f32; 2]>,
        held: bool,
        pressed: bool,
    ) -> bool {
        if pressed && let Some(launcher::menu::MenuAction::Studio(Action::Graph(action))) = hovered
        {
            if !self.graph_context_available()
                || !self.view.camera_workspace.graph.open
                || !self.graph_publication_current()
            {
                return true;
            }
            if let GraphAction::Scrub { bounds } = action {
                if let Some([x, _]) = point {
                    let fraction = ((f64::from(x) - f64::from(bounds[0]))
                        / f64::from((bounds[2] - bounds[0]).max(1)))
                    .clamp(0.0, 1.0);
                    let at = MediaTime(
                        self.view.visible_start.0
                            + (fraction * self.view.visible_duration.0 as f64) as u64,
                    );
                    self.activate(Action::Seek(at));
                }
                return true;
            }
            if let GraphAction::Drag {
                key,
                control,
                bounds,
            } = action
            {
                let Some(point) = point else {
                    return true;
                };
                let Some(publication) = self.view.camera_workspace.graph.publication.clone() else {
                    return true;
                };
                let Ok(project) = self.project().cloned() else {
                    return true;
                };
                let locked = recording_studio::tracks::item_track(&project, key.item)
                    .is_ok_and(|track| self.view.workspace.timeline.locked.contains(&track));
                if locked
                    || self.view.selected != Some(key.item)
                    || self.view.camera_workspace.graph.channel != Some(key.channel)
                    || !Arc::ptr_eq(&publication.project, &project)
                    || publication.curve.item != key.item
                    || publication.curve.channel != key.channel
                    || !publication.curve.points.iter().any(|p| p.at == key.at)
                    || (control != GraphControl::Key && publication.curve.handles(key.at).is_none())
                    || bounds[2] <= bounds[0]
                    || bounds[3] <= bounds[1]
                {
                    return true;
                }
                let origin = if control == GraphControl::Key {
                    [
                        key.at.0 as f64,
                        publication
                            .curve
                            .points
                            .iter()
                            .find(|p| p.at == key.at)
                            .unwrap()
                            .value,
                    ]
                } else {
                    publication.curve.handles(key.at).unwrap()
                        [usize::from(control == GraphControl::Incoming)]
                };
                self.release_camera();
                let inspection = Arc::make_mut(&mut self.view).camera_workspace.view.take();
                self.transport.pause();
                self.transport.seek(key.at, project.duration());
                let range = self
                    .view
                    .camera_workspace
                    .graph
                    .range
                    .unwrap_or(publication.curve.range);
                self.graph.gesture = Some(Gesture {
                    project,
                    inspection,
                    curve: publication.curve,
                    key,
                    control,
                    bounds,
                    down: point,
                    origin,
                    last: point,
                    window: [self.view.visible_start, self.view.visible_duration],
                    range,
                    pending: None,
                    result_key: key,
                    epoch: [
                        self.document,
                        self.editor.as_ref().unwrap().revision(),
                        self.transport.epoch(),
                    ],
                });
                Arc::make_mut(&mut self.view)
                    .camera_workspace
                    .graph
                    .selected = Some(key);
                self.publish_editor();
            }
        }
        let Some(mut gesture) = self.graph.gesture.take() else {
            return false;
        };
        if !gesture.current(self) {
            if self.document == gesture.epoch[0] {
                Arc::make_mut(&mut self.view).camera_workspace.view = gesture.inspection;
            }
            self.graph.project = None;
            self.publish_editor();
            return true;
        }
        if !held {
            self.graph.project = None;
            if let Some(command) = gesture.pending {
                if let Err(error) = self.edit("Drag curve", &[command]) {
                    let view = Arc::make_mut(&mut self.view);
                    view.camera_workspace.view = gesture.inspection;
                    view.error = Some(error);
                } else {
                    Arc::make_mut(&mut self.view)
                        .camera_workspace
                        .graph
                        .selected = Some(gesture.result_key);
                    self.transport.seek(
                        gesture.result_key.at,
                        self.project().map_or(MediaTime::ZERO, |p| p.duration()),
                    );
                }
            } else {
                Arc::make_mut(&mut self.view).camera_workspace.view = gesture.inspection;
            }
            self.publish_editor();
            return true;
        }
        if let Some(point) = point.filter(|p| p.iter().all(|v| v.is_finite()))
            && point != gesture.last
            && ((point[0] - gesture.down[0]).hypot(point[1] - gesture.down[1]) >= 3.0
                || gesture.pending.is_some())
        {
            gesture.last = point;
            let width = f64::from(gesture.bounds[2] - gesture.bounds[0]);
            let height = f64::from(gesture.bounds[3] - gesture.bounds[1]);
            let time = gesture.origin[0]
                + f64::from(point[0] - gesture.down[0]) / width * gesture.window[1].0 as f64;
            let value = graph::range_value(
                gesture.range,
                graph::range_fraction(gesture.range, gesture.origin[1])
                    - f64::from(point[1] - gesture.down[1]) / height,
            );
            let command = if gesture.control == GraphControl::Key {
                let at = MediaTime(
                    time.clamp(
                        gesture.curve.span.start.0 as f64,
                        gesture.curve.span.end.0 as f64,
                    )
                    .round() as u64,
                );
                let at = if point[0] == gesture.down[0] {
                    gesture.key.at
                } else {
                    gesture
                        .project
                        .export
                        .fps
                        .snap(at)
                        .unwrap_or(at)
                        .clamp(gesture.curve.span.start, gesture.curve.span.end)
                };
                graph::set_key_on(
                    &gesture.curve,
                    &gesture.project,
                    Some(gesture.key.at),
                    at,
                    value,
                )
            } else {
                let index = gesture
                    .curve
                    .points
                    .iter()
                    .position(|p| p.at == gesture.key.at)
                    .unwrap_or(0);
                if index == 0 {
                    self.graph.gesture = Some(gesture);
                    return true;
                }
                let b = gesture.curve.points[index];
                let Some(normalized) = gesture.curve.handle_input(gesture.key.at, [time, value])
                else {
                    self.graph.gesture = Some(gesture);
                    return true;
                };
                graph::with_handle(b.easing, gesture.control, normalized).and_then(|ease| {
                    graph::set_easing_on(&gesture.curve, &gesture.project, gesture.key, ease)
                })
            };
            match command {
                Ok(command) => {
                    let draft = self
                        .editor
                        .as_ref()
                        .unwrap()
                        .draft(std::slice::from_ref(&command));
                    match draft {
                        Ok(project) => {
                            self.graph.project = Some(Arc::new(project));
                            gesture.pending = Some(command);
                        }
                        Err(error) => Arc::make_mut(&mut self.view).error = Some(error.to_string()),
                    }
                }
                Err(error) => Arc::make_mut(&mut self.view).error = Some(error.to_string()),
            }
        }
        self.graph.gesture = Some(gesture);
        self.publish_editor();
        let result = self.graph.gesture.as_ref().and_then(|gesture| {
            let index = gesture
                .curve
                .points
                .iter()
                .position(|p| p.at == gesture.key.at)?;
            let curve = &self.view.camera_workspace.graph.publication.as_ref()?.curve;
            let point = curve.points.get(index)?;
            Some(GraphKey {
                at: point.at,
                ..gesture.key
            })
        });
        if let Some(key) = result {
            if let Some(gesture) = &mut self.graph.gesture {
                gesture.result_key = key;
            }
            Arc::make_mut(&mut self.view)
                .camera_workspace
                .graph
                .selected = Some(key);
        }
        true
    }
}
