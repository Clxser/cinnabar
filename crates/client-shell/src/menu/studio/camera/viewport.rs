//! Focused replay navigation and pointer gestures keep one immutable source shot until commit.

use super::super::*;
use recording_studio::camera::{
    CameraDragAxis, CameraPose, CameraProjection, CameraRigKind, CameraShot, CameraSnap,
    CameraTargetRef, CameraView, PathControl,
};

const DRAG_THRESHOLD: f32 = 4.0;

#[derive(Default)]
pub(in crate::menu::studio) struct CameraInteraction {
    pub(in crate::menu::studio) project: Option<Arc<Project>>,
    gesture: Option<Gesture>,
    revision: u64,
}

enum Gesture {
    Orbit {
        down: [f32; 2],
        pose: CameraPose,
        pivot: [f64; 3],
        last: Option<[f32; 2]>,
    },
    Control {
        project: Arc<Project>,
        shot: CameraShot,
        at: MediaTime,
        control: PathControl,
        down: [f32; 2],
        bounds: [i32; 4],
        projection: CameraProjection,
        origin: [f64; 3],
        axis: CameraDragAxis,
        snap: CameraSnap,
        pending: Option<CameraShot>,
    },
    Press {
        down: [f32; 2],
        subject: Option<CameraTargetRef>,
    },
}

impl Studio {
    /// An inspected pose publishes a unique view revision without seeking or editing the document.
    pub(in crate::menu::studio) fn inspect_camera_pose(&mut self, pose: CameraPose) {
        self.release_camera();
        self.camera_interaction.revision = self.camera_interaction.revision.wrapping_add(1);
        let workspace = &mut Arc::make_mut(&mut self.view).camera_workspace;
        workspace.focused = true;
        workspace.view = Some(CameraView {
            pose,
            revision: self.camera_interaction.revision,
        });
    }

    /// A document edit invalidates a captured source shot; deleted or deselected keys lose their handles.
    pub(in crate::menu::studio) fn reconcile_camera_workspace(&mut self) {
        let changed = match self.camera_interaction.gesture.as_ref() {
            Some(Gesture::Control { project, .. }) => self
                .project()
                .map_or(true, |current| !Arc::ptr_eq(current, project)),
            _ => false,
        };
        if changed {
            self.release_camera();
        }
        if self.view.camera_workspace.key.is_some_and(|(id, at)| {
            self.view.selected != Some(id)
                || self.project().map_or(true, |project| {
                    !project
                        .shots
                        .iter()
                        .any(|shot| shot.id == id && shot.key_at(at).is_some())
                })
        }) {
            Arc::make_mut(&mut self.view).camera_workspace.key = None;
        }
        self.reconcile_camera_path_cache();
        self.reconcile_saved_camera_views();
        self.reconcile_camera_subjects();
    }

    /// Inspector geometry is prepared once on selection/document/draft changes, never on seek or camera movement.
    fn reconcile_camera_path_cache(&mut self) {
        let Some(editor) = self.editor.as_ref() else {
            if self.view.camera_workspace.path_cache.is_some() {
                Arc::make_mut(&mut self.view).camera_workspace.path_cache = None;
            }
            return;
        };
        let project = editor.project();
        let draft = self.view.camera_workspace.draft.as_ref();
        let shot = draft.map(Arc::as_ref).or_else(|| {
            project
                .shots
                .iter()
                .find(|shot| Some(shot.id) == self.view.selected)
                .or_else(|| {
                    project
                        .shots
                        .iter()
                        .filter(|shot| shot.span.contains(self.transport.position()))
                        .max_by_key(|shot| (shot.span.start, shot.id))
                })
        });
        let Some(shot) = shot.filter(|shot| {
            shot.path_timing != recording_studio::camera::PathTiming::Keyed
                && shot.path_keys().len() > 1
        }) else {
            if self.view.camera_workspace.path_cache.is_some() {
                Arc::make_mut(&mut self.view).camera_workspace.path_cache = None;
            }
            return;
        };
        let epoch = [self.document, editor.revision()];
        if self
            .view
            .camera_workspace
            .path_cache
            .as_ref()
            .is_some_and(|cache| cache.matches(shot.id, epoch, draft))
        {
            return;
        }
        let cache = recording_studio::camera::CompiledCameraPath::new(shot).map(|path| {
            launcher::menu::studio::CameraPathCache {
                shot: shot.id,
                epoch,
                draft: draft.cloned(),
                path: Arc::new(path),
            }
        });
        Arc::make_mut(&mut self.view).camera_workspace.path_cache = cache;
    }

    /// Keyboard focus uses the displayed replay pose and pauses the transport before moving it.
    pub(in crate::menu) fn focus_camera(&mut self) -> Result<(), String> {
        if self.view.camera_workspace.focused && self.view.camera_workspace.view.is_some() {
            self.transport.pause();
            return Ok(());
        }
        let preview = self.camera_preview()?;
        if !preview.interactive {
            return Err("Direct camera editing requires one unframed native replay view.".into());
        }
        self.transport.pause();
        let workspace = &mut Arc::make_mut(&mut self.view).camera_workspace;
        workspace.focused = true;
        if workspace.view.is_none() {
            self.camera_interaction.revision = self.camera_interaction.revision.wrapping_add(1);
            workspace.view = Some(CameraView {
                pose: preview.render_pose,
                revision: self.camera_interaction.revision,
            });
        }
        Ok(())
    }
    /// Focus loss, hidden panels and Escape abandon the uncommitted gesture while retaining the view.
    pub(in crate::menu) fn release_camera(&mut self) {
        self.camera_interaction.gesture = None;
        self.camera_interaction.project = None;
        let workspace = &mut Arc::make_mut(&mut self.view).camera_workspace;
        workspace.focused = false;
        workspace.looking = false;
        workspace.draft = None;
    }

    /// Reset returns to the evaluated timeline camera without changing saved camera metadata.
    pub(in crate::menu) fn reset_camera_view(&mut self) {
        self.release_camera();
        Arc::make_mut(&mut self.view).camera_workspace.view = None;
    }

    /// Pointer capture survives leaving the painted image and commits one replacement on release.
    pub(in crate::menu) fn camera_pointer(
        &mut self,
        hovered: Option<launcher::menu::MenuAction>,
        point: Option<[f32; 2]>,
        held: bool,
        pressed: bool,
    ) -> bool {
        if pressed {
            let action = match hovered {
                Some(launcher::menu::MenuAction::Studio(action)) => action,
                _ => {
                    self.release_camera();
                    return false;
                }
            };
            if !matches!(
                action,
                Action::CameraViewport { .. }
                    | Action::CameraControl { .. }
                    | Action::CameraAxisControl { .. }
                    | Action::CameraSubject(_)
            ) {
                self.release_camera();
                return false;
            }
            if let Err(error) = self.begin_camera_pointer(action, point) {
                self.release_camera();
                Arc::make_mut(&mut self.view).error = Some(error);
                return true;
            }
        }
        let Some(mut gesture) = self.camera_interaction.gesture.take() else {
            return false;
        };
        if !held {
            self.camera_interaction.project = None;
            Arc::make_mut(&mut self.view).camera_workspace.draft = None;
            let result = match gesture {
                Gesture::Control {
                    project,
                    shot: original,
                    pending: Some(shot),
                    ..
                } if shot != original => {
                    if self
                        .project()
                        .is_ok_and(|current| Arc::ptr_eq(current, &project))
                    {
                        self.edit("Drag camera path control", &[Command::ReplaceShot { shot }])
                    } else {
                        Err("The project changed during the camera drag. Try again.".into())
                    }
                }
                Gesture::Press {
                    down,
                    subject: Some(reference),
                    ..
                } if point.is_some_and(|point| {
                    distance_squared(point, down) < DRAG_THRESHOLD * DRAG_THRESHOLD
                }) =>
                {
                    self.select_camera_subject(reference)
                }
                _ => Ok(()),
            };
            if let Err(error) = result {
                Arc::make_mut(&mut self.view).error = Some(error);
            }
            self.publish_editor();
            return true;
        }
        if let Gesture::Control {
            project,
            shot,
            at,
            control,
            down,
            bounds,
            projection,
            origin,
            axis,
            snap,
            pending,
        } = &mut gesture
            && let Some(point) = point.filter(|point| point.iter().all(|axis| axis.is_finite()))
            && (pending.is_some()
                || distance_squared(point, *down) >= DRAG_THRESHOLD * DRAG_THRESHOLD)
        {
            let delta = [
                f64::from(point[0] - down[0]) / f64::from((bounds[2] - bounds[0]).max(1)),
                f64::from(point[1] - down[1]) / f64::from((bounds[3] - bounds[1]).max(1)),
            ];
            if let Some(position) = projection.dragged_constrained(*origin, delta, *axis, *snap) {
                match shot.with_path_control(*at, *control, position) {
                    Ok(next) if pending.as_ref() != Some(&next) => {
                        let mut draft = (**project).clone();
                        if let Some(replacement) =
                            draft.shots.iter_mut().find(|item| item.id == next.id)
                        {
                            *replacement = next.clone();
                        }
                        self.camera_interaction.project = Some(Arc::new(draft));
                        Arc::make_mut(&mut self.view).camera_workspace.draft =
                            Some(Arc::new(next.clone()));
                        *pending = Some(next);
                    }
                    Err(error) => Arc::make_mut(&mut self.view).error = Some(error.to_string()),
                    _ => {}
                }
            }
        }
        if let Gesture::Orbit {
            down,
            pose,
            pivot,
            last,
        } = &mut gesture
            && let Some(point) = point.filter(|point| point.iter().all(|axis| axis.is_finite()))
            && *last != Some(point)
            && (last.is_some() || distance_squared(point, *down) >= DRAG_THRESHOLD * DRAG_THRESHOLD)
        {
            let look = [
                f64::from(point[0] - down[0]) * 0.22,
                f64::from(point[1] - down[1]) * 0.22,
            ];
            if let Some(next) = pose.orbited(*pivot, look) {
                self.camera_interaction.revision = self.camera_interaction.revision.wrapping_add(1);
                Arc::make_mut(&mut self.view).camera_workspace.view = Some(CameraView {
                    pose: next,
                    revision: self.camera_interaction.revision,
                });
                *last = Some(point);
            }
        }
        self.camera_interaction.gesture = Some(gesture);
        true
    }

    /// Starts only from a current native replay image; no live entity or camera is consulted.
    fn begin_camera_pointer(
        &mut self,
        action: Action,
        point: Option<[f32; 2]>,
    ) -> Result<(), String> {
        let preview = self.camera_preview()?;
        if !preview.interactive {
            return Err("Direct camera editing requires one unframed native replay view.".into());
        }
        let down = point.ok_or("The pointer is outside the Studio viewport.")?;
        let action = if let Action::CameraAxisControl {
            shot,
            at,
            handle,
            axis,
            bounds,
        } = action
        {
            Arc::make_mut(&mut self.view).camera_workspace.drag_axis = axis;
            Action::CameraControl {
                shot,
                at,
                handle,
                bounds,
            }
        } else {
            action
        };
        self.transport.pause();
        self.cancel_camera_value();
        self.cancel_clip_time();
        self.cancel_title_text();
        self.camera_interaction.revision = self.camera_interaction.revision.wrapping_add(1);
        let workspace = &mut Arc::make_mut(&mut self.view).camera_workspace;
        workspace.focused = true;
        if workspace.view.is_none() {
            workspace.view = Some(CameraView {
                pose: preview.render_pose,
                revision: self.camera_interaction.revision,
            });
        }
        self.camera_interaction.gesture = Some(match action {
            Action::CameraControl {
                shot: id,
                at,
                handle,
                bounds,
            } => {
                let project = Arc::clone(self.project()?);
                let shot = project
                    .shots
                    .iter()
                    .find(|shot| shot.id == id)
                    .cloned()
                    .ok_or("This camera shot was removed.")?;
                let origin = shot
                    .path_control(at, handle)
                    .ok_or("This camera path control is no longer available.")?;
                if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
                    return Err("The camera viewport is unavailable.".into());
                }
                let projection = CameraProjection::new(
                    preview.render_pose,
                    f64::from(bounds[2] - bounds[0]) / f64::from(bounds[3] - bounds[1]),
                )
                .ok_or("The displayed camera cannot be projected.")?;
                self.select_camera_key(id, at)?;
                let workspace = &mut Arc::make_mut(&mut self.view).camera_workspace;
                workspace.key = Some((id, at));
                workspace.view = Some(CameraView {
                    pose: preview.render_pose,
                    revision: self.camera_interaction.revision,
                });
                Gesture::Control {
                    project,
                    shot,
                    at,
                    control: handle,
                    down,
                    bounds,
                    projection,
                    origin,
                    axis: workspace.drag_axis,
                    snap: workspace.snap,
                    pending: None,
                }
            }
            Action::CameraSubject(reference) => Gesture::Press {
                down,
                subject: Some(reference),
            },
            Action::CameraViewport { .. } => {
                let pose = self
                    .view
                    .camera_workspace
                    .view
                    .map_or(preview.render_pose, |view| view.pose);
                let pivot = self
                    .view
                    .camera_target
                    .and_then(|reference| {
                        preview
                            .targets
                            .iter()
                            .find(|target| target.matches(reference))
                    })
                    .map(|target| {
                        std::array::from_fn(|axis| {
                            target.pose.position[axis]
                                + target.pose.eye_offset.unwrap_or([0.0; 3])[axis] * 0.5
                        })
                    })
                    .or_else(|| pose.orbit_anchor(8.0))
                    .ok_or("The inspected camera has no valid orbit pivot.")?;
                Gesture::Orbit {
                    down,
                    pose,
                    pivot,
                    last: None,
                }
            }
            _ => unreachable!(),
        });
        self.publish_editor();
        Ok(())
    }

    /// Navigation stays transient; changing the view does not create undo entries or seek replay state.
    pub(in crate::menu) fn navigate_camera(
        &mut self,
        movement: [f64; 3],
        look: [f64; 2],
        elapsed: f64,
        fast: bool,
        looking: bool,
    ) {
        if !self.view.camera_workspace.focused
            || self
                .camera_interaction
                .gesture
                .as_ref()
                .is_some_and(|gesture| {
                    matches!(gesture, Gesture::Control { .. } | Gesture::Orbit { .. })
                })
        {
            return;
        }
        let Some(current) = self.view.camera_workspace.view else {
            return;
        };
        Arc::make_mut(&mut self.view).camera_workspace.looking = looking;
        let Some(pose) = current
            .pose
            .flown(movement, look, elapsed, if fast { 32.0 } else { 8.0 })
        else {
            return;
        };
        if pose == current.pose {
            return;
        }
        self.camera_interaction.revision = self.camera_interaction.revision.wrapping_add(1);
        Arc::make_mut(&mut self.view).camera_workspace.view = Some(CameraView {
            pose,
            revision: self.camera_interaction.revision,
        });
    }

    /// Clicked subjects must still belong to the recorded frame and the exact saved spawn lifetime.
    pub(in crate::menu) fn select_camera_subject(
        &mut self,
        reference: CameraTargetRef,
    ) -> Result<(), String> {
        let preview = self.displayed_camera_preview()?;
        let target = preview
            .targets
            .iter()
            .find(|target| {
                target.matches(reference)
                    && self
                        .view
                        .camera_workspace
                        .subject_matches(target, preview.local_actor)
            })
            .ok_or("This recorded subject is no longer present.")?;
        Arc::make_mut(&mut self.view).camera_target = Some(target.reference());
        Ok(())
    }

    /// Applying a fly view authors a free shot or replaces one selected path pose in one undo step.
    pub(in crate::menu::studio) fn apply_camera_view(&mut self) -> Result<(), String> {
        let inspected = self
            .view
            .camera_workspace
            .view
            .ok_or("Focus and move the replay view first.")?
            .pose;
        let shot = self.camera_shot().ok().filter(|shot| {
            matches!(
                shot.kind(),
                CameraRigKind::Free | CameraRigKind::Path | CameraRigKind::Track
            )
        });
        if let Some(shot) = &shot {
            shot.ensure_pose_keying()
                .map_err(|error| error.to_string())?;
        }
        let at = shot
            .as_ref()
            .and_then(|shot| {
                self.view
                    .camera_workspace
                    .key
                    .filter(|(id, _)| *id == shot.id)
            })
            .map_or(self.transport.position(), |(_, at)| at);
        let pose = self.camera_authoring_pose(at, inspected)?;
        if let Some(shot) = shot {
            let shot = shot
                .with_pose(at, pose)
                .map_err(|error| error.to_string())?;
            self.edit("Apply camera view", &[Command::ReplaceShot { shot }])?;
        } else {
            let preview = self.camera_preview()?;
            let project = self.project()?;
            let at = self.transport.position();
            let end = project
                .clips
                .iter()
                .filter(|clip| clip.enabled && clip.source == preview.source)
                .filter_map(|clip| clip.span().ok())
                .filter(|span| span.contains(at))
                .map(|span| span.end)
                .max()
                .ok_or("The view is outside a native replay clip.")?;
            let span = Span::new(at, MediaTime(end.0.min(at.0.saturating_add(6_000_000))))
                .map_err(|error| error.to_string())?;
            let id = project.next_id().map_err(|error| error.to_string())?;
            let shot = CameraShot::from_preview(id, span, CameraRigKind::Free, pose, None)
                .map_err(|error| error.to_string())?;
            self.edit("Apply camera view", &[Command::AddShot { shot }])?;
            Arc::make_mut(&mut self.view).selected = Some(id);
        }
        self.reset_camera_view();
        Ok(())
    }

    /// An inspection pose is authored once beneath effects; ordinary previews already supply a base pose.
    pub(in crate::menu::studio) fn camera_authoring_pose(
        &self,
        at: MediaTime,
        base: recording_studio::camera::CameraPose,
    ) -> Result<recording_studio::camera::CameraPose, String> {
        let Some(inspected) = self.view.camera_workspace.view else {
            return Ok(base);
        };
        let timeline =
            recording_studio::playback::CompiledTimeline::new(Arc::clone(self.project()?))
                .map_err(|error| error.to_string())?;
        let mut plan = recording_studio::playback::FramePlan::default();
        timeline
            .sample(at, &mut plan, |_, _, _| None)
            .map_err(|error| error.to_string())?;
        plan.effects
            .camera_base_pose(inspected.pose, at)
            .map_err(|error| {
                format!(
                    "This view cannot be applied under the current shake/zoom settings: {error}"
                )
            })
    }

    /// Restores automatic handles for the selected key while preserving the shot's other controls.
    pub(in crate::menu::studio) fn reset_camera_handles(&mut self) -> Result<(), String> {
        let shot = self.camera_shot()?;
        let at = self
            .view
            .camera_workspace
            .key
            .filter(|(id, _)| *id == shot.id)
            .map_or(self.transport.position(), |(_, at)| at);
        let shot = shot
            .without_path_handles(at)
            .map_err(|error| error.to_string())?;
        self.edit("Reset camera handles", &[Command::ReplaceShot { shot }])
    }
}

/// A squared logical-pixel threshold distinguishes selecting a key from changing its geometry.
fn distance_squared(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}
