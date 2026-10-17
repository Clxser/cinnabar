//! Timeline gestures keep project changes transactional and navigation independent of undo history.

use super::*;
use launcher::menu::studio::EditorPanel;
use recording_studio::timeline::{DragMode, TimelineEdit};

pub(super) enum Gesture {
    Overview {
        left: i32,
        width: u32,
    },
    Edit {
        id: Id,
        mode: DragMode,
        width: u32,
        down: f32,
        duration: MediaTime,
        project: Arc<Project>,
        pending: Option<TimelineEdit>,
    },
}

impl Studio {
    /// Navigation never exposes a time window beyond the project plus a small insertion margin.
    fn timeline_end(&self) -> u64 {
        let duration = self
            .view
            .project
            .as_ref()
            .map_or(MediaTime::ZERO, |project| project.duration());
        recording_studio::timeline::overview_end(duration).0
    }

    /// Horizontal wheel units retain trackpad precision instead of rounding to button-sized jumps.
    pub(super) fn pan_timeline(&mut self, fraction: f64) {
        if !fraction.is_finite() {
            return;
        }
        let maximum = self
            .timeline_end()
            .saturating_sub(self.view.visible_duration.0);
        let start =
            self.view.visible_start.0 as f64 + fraction * self.view.visible_duration.0 as f64;
        Arc::make_mut(&mut self.view).visible_start =
            MediaTime(start.clamp(0.0, maximum as f64).round() as u64);
    }

    /// Zoom retains the time under the pointer, even at fractional wheel steps and project edges.
    pub(super) fn zoom_timeline(&mut self, steps: f64, anchor: f64) {
        if !steps.is_finite() || !anchor.is_finite() {
            return;
        }
        let anchor = anchor.clamp(0.0, 1.0);
        let old = self.view.visible_duration.0;
        let minimum = self
            .view
            .project
            .as_ref()
            .and_then(|project| project.export.fps.frame_time(8).ok())
            .map_or(100_000, |time| time.0.max(1_000));
        let end = self.timeline_end();
        let duration = ((old as f64 * 2.0_f64.powf(-steps.clamp(-12.0, 12.0))) as u64)
            .clamp(minimum.min(end), end);
        let at = self.view.visible_start.0 as f64 + old as f64 * anchor;
        let start = (at - duration as f64 * anchor).clamp(0.0, end.saturating_sub(duration) as f64);
        let view = Arc::make_mut(&mut self.view);
        view.visible_duration = MediaTime(duration);
        view.visible_start = MediaTime(start.round() as u64);
    }

    /// Fits the whole admitted project without changing the edit or playback position.
    pub(super) fn fit_timeline(&mut self) {
        let end = self.timeline_end();
        let view = Arc::make_mut(&mut self.view);
        view.visible_start = MediaTime::ZERO;
        view.visible_duration = MediaTime(end);
    }

    /// Explicit reveal keeps manual panning useful while making a lost playhead easy to find.
    pub(super) fn reveal_playhead(&mut self) {
        let maximum = self
            .timeline_end()
            .saturating_sub(self.view.visible_duration.0);
        let start = self
            .transport
            .position()
            .0
            .saturating_sub(self.view.visible_duration.0 / 3)
            .min(maximum);
        Arc::make_mut(&mut self.view).visible_start = MediaTime(start);
    }

    /// The caller supplies the last painted scroll viewport in window-logical pixels.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::menu) fn timeline_wheel(
        &mut self,
        point: ui::UiPoint,
        viewport: Option<ui::UiRect>,
        ruler: bool,
        horizontal: f32,
        vertical: f32,
        pixels: bool,
        shift: bool,
        zoom: bool,
    ) -> bool {
        if self.view.show_library || self.view.workflow.show_settings || self.view.project.is_none()
        {
            return false;
        }
        let Some(viewport) = viewport else {
            return false;
        };
        if !viewport.contains(point) && !ruler {
            return false;
        }
        if !horizontal.is_finite() || !vertical.is_finite() {
            return false;
        }
        let width = (viewport.max().x() - viewport.min().x()).max(1.0);
        if zoom {
            let steps = f64::from(vertical) / if pixels { 160.0 } else { 4.0 };
            self.zoom_timeline(steps, f64::from((point.x() - viewport.min().x()) / width));
            return true;
        }
        if horizontal != 0.0 || shift || ruler {
            let amount = if horizontal != 0.0 {
                horizontal
            } else {
                vertical
            };
            let fraction = -f64::from(amount) / if pixels { f64::from(width) } else { 12.0 };
            self.pan_timeline(fraction);
            return true;
        }
        false
    }

    /// One project snapshot supplies the entire drag; releasing commits once, focus loss discards it.
    pub(super) fn timeline_pointer(
        &mut self,
        hovered: Option<launcher::menu::MenuAction>,
        x: Option<f32>,
        held: bool,
        pressed: bool,
    ) -> bool {
        if !held {
            let Some(gesture) = self.gesture.take() else {
                return false;
            };
            Arc::make_mut(&mut self.view).workspace.drag = None;
            if let Gesture::Edit {
                project,
                pending: Some(edit),
                ..
            } = gesture
            {
                let result = if self
                    .project()
                    .is_ok_and(|current| Arc::ptr_eq(current, &project))
                {
                    self.edit("Drag timeline item", &edit.commands)
                } else {
                    Err("The project changed during this drag. Try dragging again.".into())
                };
                if let Err(error) = result {
                    Arc::make_mut(&mut self.view).error = Some(error);
                }
                self.publish_editor();
            }
            return true;
        }
        if pressed && let Some(launcher::menu::MenuAction::Studio(action)) = hovered {
            match action {
                Action::TimelineOverview { left, width } => {
                    self.gesture = Some(Gesture::Overview { left, width })
                }
                Action::DragItem {
                    id, mode, width, ..
                } => {
                    if let (Some(down), Ok(project)) = (x, self.project().cloned()) {
                        self.gesture = Some(Gesture::Edit {
                            id,
                            mode,
                            width,
                            down,
                            duration: self.view.visible_duration,
                            project,
                            pending: None,
                        });
                        self.transport.pause();
                        if let Err(error) = self.select_timeline_item(id) {
                            Arc::make_mut(&mut self.view).error = Some(error);
                        }
                        Arc::make_mut(&mut self.view).workspace.panel = EditorPanel::Inspector;
                        self.publish_editor();
                    }
                }
                _ => {}
            }
        }
        let Some(gesture) = self.gesture.take() else {
            return false;
        };
        match gesture {
            Gesture::Overview { left, width } => {
                if let Some(x) = x {
                    let at = ((f64::from(x) - f64::from(left)) / f64::from(width.max(1)))
                        .clamp(0.0, 1.0)
                        * self.timeline_end() as f64;
                    let maximum = self
                        .timeline_end()
                        .saturating_sub(self.view.visible_duration.0);
                    let start =
                        (at - self.view.visible_duration.0 as f64 * 0.5).clamp(0.0, maximum as f64);
                    Arc::make_mut(&mut self.view).visible_start = MediaTime(start.round() as u64);
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
                mut pending,
            } => {
                if let Some(x) = x.filter(|value| value.is_finite()) {
                    let delta = ((f64::from(x - down) / f64::from(width.max(1)))
                        * duration.0 as f64) as i128;
                    let fps = project.export.fps;
                    let span = recording_studio::timeline::item_span(&project, id);
                    if let Ok(span) = span {
                        let anchor = if mode == DragMode::TrimEnd {
                            span.end
                        } else {
                            span.start
                        };
                        let wanted = MediaTime(
                            (i128::from(anchor.0) + delta)
                                .clamp(0, i128::from(recording_studio::time::MAX_MEDIA_MICROS))
                                as u64,
                        );
                        if let Ok(snapped) = fps.snap(wanted) {
                            let delta = i128::from(snapped.0) - i128::from(anchor.0);
                            if let Ok(edit) = recording_studio::timeline::drag(
                                &project,
                                id,
                                mode,
                                delta,
                                self.view.workspace.ripple,
                            ) {
                                Arc::make_mut(&mut self.view).workspace.drag =
                                    Some((id, edit.span));
                                pending = Some(edit);
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
                    pending,
                });
            }
        }
        true
    }
}
