//! Captured timeline gestures use immutable starting metadata and commit exactly once.
use super::*;
use launcher::menu::studio::timeline::TimelineAction;

impl Studio {
    /// Releasing commits one frozen-project edit; focus loss uses the separate cancellation path.
    pub(in super::super) fn timeline_pointer(
        &mut self,
        hovered: Option<launcher::menu::MenuAction>,
        point: Option<[f32; 2]>,
        held: bool,
        pressed: bool,
        modifiers: [bool; 3],
    ) -> bool {
        if !held {
            let Some(gesture) = self.gesture.take() else {
                return false;
            };
            self.clear_timeline_drag();
            Arc::make_mut(&mut self.view).workspace.drag = None;
            let (project, revision, commands, key) = match gesture {
                Gesture::Edit {
                    project,
                    revision,
                    pending: Some(commands),
                    ..
                } => (project, revision, commands, None),
                Gesture::Key {
                    project,
                    revision,
                    key,
                    pending,
                    ..
                } => {
                    let (commands, key) =
                        pending.map_or((Vec::new(), key), |(command, key)| (vec![command], key));
                    (project, revision, commands, Some(key))
                }
                _ => return true,
            };
            let result = if self
                .project()
                .is_ok_and(|current| Arc::ptr_eq(current, &project))
                && self
                    .editor
                    .as_ref()
                    .is_some_and(|editor| editor.revision() == revision)
            {
                self.edit("Drag timeline selection", &commands)
                    .and_then(|()| {
                        if let Some(key) = key {
                            let at = recording_studio::timeline::key_time(self.project()?, key)
                                .map_err(|error| error.to_string())?;
                            let duration = self.project()?.duration();
                            self.transport.seek(at, duration);
                            if key.kind == recording_studio::timeline::KeyKind::Speed {
                                Arc::make_mut(&mut self.view).speed_key = Some((key.item, key.at));
                            }
                        }
                        Ok(())
                    })
            } else {
                Err("The project changed during this drag. Try dragging again.".into())
            };
            if let Err(error) = result {
                Arc::make_mut(&mut self.view).error = Some(error);
            }
            self.publish_editor();
            return true;
        }
        if pressed && let Some(launcher::menu::MenuAction::Studio(action)) = hovered {
            match action {
                Action::TimelineOverview { left, width } => {
                    self.gesture = Some(Gesture::Overview { left, width })
                }
                Action::Select(id) | Action::DragItem { id, .. } => {
                    if !self.project().is_ok_and(|project| {
                        recording_studio::timeline::item_span(project, id).is_ok()
                    }) {
                        return false;
                    }
                    let selected = self.select_group(id, modifiers[0], modifiers[1]);
                    if let Err(error) = &selected {
                        Arc::make_mut(&mut self.view).error = Some(error.clone());
                    }
                    self.publish_editor();
                    if modifiers[1] || selected != Ok(true) {
                        return true;
                    }
                    if let (Some([down, _]), Ok(project), Action::DragItem { mode, width, .. }) =
                        (point, self.project().cloned(), action)
                    {
                        let locked = &self.view.workspace.timeline.locked;
                        let selected = if mode == DragMode::Move {
                            self.timeline_selection()
                        } else {
                            BTreeSet::from([id])
                        };
                        if selected.iter().any(|id| {
                            recording_studio::tracks::item_track(&project, *id)
                                .is_ok_and(|track| locked.contains(&track))
                        }) {
                            return true;
                        }
                        let edges = selected
                            .iter()
                            .filter_map(|id| {
                                recording_studio::timeline::item_span(&project, *id).ok()
                            })
                            .flat_map(|span| {
                                if mode == DragMode::TrimStart {
                                    vec![span.start]
                                } else if mode == DragMode::TrimEnd {
                                    vec![span.end]
                                } else {
                                    vec![span.start, span.end]
                                }
                            })
                            .collect();
                        let snap = SnapIndex::new(&project, &selected, self.transport.position());
                        self.gesture = Some(Gesture::Edit {
                            id,
                            mode,
                            width,
                            down,
                            duration: self.view.visible_duration,
                            project,
                            revision: self.editor.as_ref().map_or(0, Editor::revision),
                            selected,
                            edges,
                            snap,
                            last: None,
                            pending: None,
                        });
                        self.transport.pause();
                        Arc::make_mut(&mut self.view).workspace.panel = EditorPanel::Inspector;
                    } else {
                        return true;
                    }
                }
                Action::Timeline(TimelineAction::DragKey {
                    key, width, height, ..
                }) => {
                    if let (Some(down), Ok(project)) = (point, self.project().cloned()) {
                        if recording_studio::tracks::item_track(&project, key.item)
                            .is_ok_and(|track| self.view.workspace.timeline.locked.contains(&track))
                        {
                            return true;
                        }
                        let _ = self.select_group(key.item, false, false);
                        self.transport.pause();
                        let snap = SnapIndex::new(
                            &project,
                            &BTreeSet::from([key.item]),
                            self.transport.position(),
                        );
                        self.gesture = Some(Gesture::Key {
                            key,
                            width,
                            height,
                            down,
                            duration: self.view.visible_duration,
                            project,
                            revision: self.editor.as_ref().map_or(0, Editor::revision),
                            snap,
                            last: None,
                            pending: None,
                        });
                        self.publish_editor();
                    }
                }
                _ => {}
            }
        }
        let Some(gesture) = self.gesture.take() else {
            return false;
        };
        let snapping = self.view.workspace.timeline.snapping && !modifiers[2];
        match gesture {
            Gesture::Overview { left, width } => {
                if let Some([x, _]) = point {
                    let at = ((f64::from(x) - f64::from(left)) / f64::from(width.max(1)))
                        .clamp(0.0, 1.0)
                        * self.timeline_end() as f64;
                    let maximum = self
                        .timeline_end()
                        .saturating_sub(self.view.visible_duration.0);
                    Arc::make_mut(&mut self.view).visible_start = MediaTime(
                        (at - self.view.visible_duration.0 as f64 * 0.5)
                            .clamp(0.0, maximum as f64)
                            .round() as u64,
                    );
                }
                self.gesture = Some(Gesture::Overview { left, width });
            }
            Gesture::Edit {
                id,
                mode,
                width,
                down,
                duration,
                project,
                revision,
                selected,
                edges,
                snap,
                mut last,
                mut pending,
            } => {
                let changed = point.is_some_and(|point| last != Some((point, snapping)));
                if changed {
                    last = point.map(|point| (point, snapping));
                }
                if changed
                    && let Some([x, _]) =
                        point.filter(|point| point.iter().all(|value| value.is_finite()))
                    && ((x - down).abs() >= 3.0 || pending.is_some())
                {
                    let raw = ((f64::from(x - down) / f64::from(width.max(1))) * duration.0 as f64)
                        as i128;
                    let anchor = edges.first().copied().unwrap_or_default();
                    let target = MediaTime(
                        (i128::from(anchor.0) + raw)
                            .clamp(0, i128::from(recording_studio::time::MAX_MEDIA_MICROS))
                            as u64,
                    );
                    if let Ok(at) = if raw == 0 {
                        Ok(anchor)
                    } else {
                        project.export.fps.snap(target)
                    } {
                        let delta = i128::from(at.0) - i128::from(anchor.0);
                        let threshold =
                            MediaTime(duration.0.saturating_mul(8) / u64::from(width.max(1)));
                        let (delta, line) = if snapping {
                            snap.delta(&edges, delta, threshold)
                        } else {
                            (delta, None)
                        };
                        let edit = if mode == DragMode::Move {
                            recording_studio::timeline::move_selection(&project, &selected, delta)
                        } else {
                            recording_studio::timeline::drag(
                                &project,
                                id,
                                mode,
                                delta,
                                self.view.workspace.ripple,
                            )
                            .map(|edit| (edit.commands, delta))
                        };
                        match edit {
                            Ok((commands, actual)) => {
                                let ripple = self.view.workspace.ripple;
                                let view = Arc::make_mut(&mut self.view);
                                view.workspace.timeline.snap_line =
                                    (actual == delta).then_some(line).flatten();
                                view.workspace.timeline.ghost = selected
                                    .iter()
                                    .filter_map(|id| {
                                        let span =
                                            recording_studio::timeline::item_span(&project, *id)
                                                .ok()?;
                                        let span = if mode == DragMode::Move {
                                            Span::new(
                                                MediaTime(
                                                    (i128::from(span.start.0) + actual) as u64,
                                                ),
                                                MediaTime((i128::from(span.end.0) + actual) as u64),
                                            )
                                            .ok()?
                                        } else {
                                            recording_studio::timeline::drag(
                                                &project, *id, mode, actual, ripple,
                                            )
                                            .ok()?
                                            .span
                                        };
                                        Some((*id, span))
                                    })
                                    .collect();
                                pending = Some(commands);
                            }
                            Err(error) => {
                                Arc::make_mut(&mut self.view).error = Some(error.to_string());
                            }
                        }
                    }
                }
                self.gesture = Some(Gesture::Edit {
                    id,
                    mode,
                    width,
                    down,
                    duration,
                    project,
                    revision,
                    selected,
                    edges,
                    snap,
                    last,
                    pending,
                });
            }
            Gesture::Key {
                key,
                width,
                height,
                down,
                duration,
                project,
                revision,
                snap,
                mut last,
                mut pending,
            } => {
                let changed = point.is_some_and(|point| last != Some((point, snapping)));
                if changed {
                    last = point.map(|point| (point, snapping));
                }
                if changed
                    && let Some(point) =
                        point.filter(|point| point.iter().all(|value| value.is_finite()))
                    && ((point[0] - down[0]).abs() >= 3.0
                        || (point[1] - down[1]).abs() >= 3.0
                        || pending.is_some())
                {
                    if let Ok(anchor) = recording_studio::timeline::key_time(&project, key) {
                        let delta = ((f64::from(point[0] - down[0]) / f64::from(width.max(1)))
                            * duration.0 as f64) as i128;
                        let wanted = MediaTime(
                            (i128::from(anchor.0) + delta)
                                .clamp(0, i128::from(recording_studio::time::MAX_MEDIA_MICROS))
                                as u64,
                        );
                        if let Ok(at) = if delta == 0 {
                            Ok(anchor)
                        } else {
                            project.export.fps.snap(wanted)
                        } {
                            let delta = i128::from(at.0) - i128::from(anchor.0);
                            let (delta, line) = if snapping {
                                snap.delta(
                                    &[anchor],
                                    delta,
                                    MediaTime(
                                        duration.0.saturating_mul(8) / u64::from(width.max(1)),
                                    ),
                                )
                            } else {
                                (delta, None)
                            };
                            let at = MediaTime(
                                (i128::from(anchor.0) + delta)
                                    .clamp(0, i128::from(recording_studio::time::MAX_MEDIA_MICROS))
                                    as u64,
                            );
                            if let Ok((command, moved)) = recording_studio::timeline::move_key(
                                &project,
                                key,
                                at,
                                f64::from(down[1] - point[1]) / (f64::from(height.max(1)) * 0.72),
                            ) {
                                let view = Arc::make_mut(&mut self.view);
                                view.workspace.timeline.snap_line = line;
                                view.workspace.timeline.key_drag =
                                    Some((key, at, (point[1] - down[1]).round() as i32));
                                pending = Some((command, moved));
                            }
                        }
                    }
                }
                self.gesture = Some(Gesture::Key {
                    key,
                    width,
                    height,
                    down,
                    duration,
                    project,
                    revision,
                    snap,
                    last,
                    pending,
                });
            }
        }
        true
    }
}
