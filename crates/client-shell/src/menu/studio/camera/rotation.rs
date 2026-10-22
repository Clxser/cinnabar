//! One captured authored rotation previews immutable metadata and commits exactly once on release.

use super::super::*;
use launcher::menu::{
    MenuAction,
    studio::{CameraRotationHandle, ViewportTool},
};
use recording_studio::camera::{CameraRotationDrag, CameraShot, CameraView};

pub(super) struct RotationGesture {
    project: Arc<Project>,
    shot: CameraShot,
    handle: CameraRotationHandle,
    epoch: [u64; 3],
    source: Id,
    down: [f32; 2],
    last: [f32; 2],
    value: f64,
    drag: CameraRotationDrag,
    pending: Option<CameraShot>,
    pub(super) inspection: Option<CameraView>,
}

impl Studio {
    /// Transient preview refreshes retain capture; document, take, seek or modal changes abandon it.
    pub(super) fn rotation_source_current(&self) -> bool {
        self.camera_interaction
            .rotation
            .as_ref()
            .is_none_or(|gesture| {
                self.camera_navigation_available()
                    && !self
                        .view
                        .workspace
                        .timeline
                        .locked
                        .contains(&recording_studio::tracks::Track::Camera)
                    && self.view.workspace.viewport_tool == ViewportTool::Rotate
                    && self.view.selected == Some(gesture.shot.id)
                    && self.transport.position() == gesture.handle.at
                    && self
                        .project()
                        .is_ok_and(|project| Arc::ptr_eq(project, &gesture.project))
                    && self.displayed_camera_preview().is_ok_and(|preview| {
                        preview.interactive
                            && preview.epoch == gesture.epoch
                            && preview.source == gesture.source
                    })
            })
    }

    /// The ring owns the original button release even outside its image, without stealing another drag.
    pub(super) fn rotation_pointer(
        &mut self,
        hovered: Option<MenuAction>,
        point: Option<[f32; 2]>,
        held: bool,
        pressed: bool,
    ) -> bool {
        if pressed && self.camera_interaction.rotation.is_none() {
            let result = match hovered {
                Some(MenuAction::Studio(Action::CameraRotationControl {
                    shot,
                    at,
                    axis,
                    bounds,
                })) => point
                    .ok_or_else(|| "Point at the camera rotation handle first.".to_owned())
                    .and_then(|down| {
                        self.begin_rotation(
                            CameraRotationHandle {
                                shot,
                                at,
                                axis,
                                bounds,
                            },
                            down,
                        )
                    }),
                _ => return false,
            };
            if let Err(error) = result {
                Arc::make_mut(&mut self.view).error = Some(error);
                return true;
            }
        }
        if !self.rotation_source_current() {
            self.release_camera();
            self.publish_editor();
            return true;
        }
        let Some(mut gesture) = self.camera_interaction.rotation.take() else {
            return false;
        };
        if !held {
            self.camera_interaction.project = None;
            let workspace = &mut Arc::make_mut(&mut self.view).camera_workspace;
            workspace.draft = None;
            workspace.rotation = None;
            let result = if let Some(shot) = gesture.pending.filter(|shot| *shot != gesture.shot) {
                // The authored preview remains on the timeline camera after an intentional commit.
                self.edit("Rotate camera", &[Command::ReplaceShot { shot }])
            } else {
                Arc::make_mut(&mut self.view).camera_workspace.view = gesture.inspection;
                Ok(())
            };
            if let Err(error) = result {
                let view = Arc::make_mut(&mut self.view);
                view.camera_workspace.view = gesture.inspection;
                view.error = Some(error);
            }
            self.publish_editor();
            return true;
        }
        let mut changed = false;
        if let Some(point) = point.filter(|point| point.iter().all(|value| value.is_finite()))
            && gesture.last != point
            && (gesture.pending.is_some()
                || (point[0] - gesture.down[0]).hypot(point[1] - gesture.down[1])
                    >= super::viewport::DRAG_THRESHOLD)
            && let Some(value) = gesture.drag.sample(point)
        {
            gesture.last = point;
            if value == gesture.value {
                self.camera_interaction.rotation = Some(gesture);
                return true;
            }
            gesture.value = value;
            match gesture
                .shot
                .with_rotation(gesture.handle.at, gesture.handle.axis, value)
            {
                Ok(next) if gesture.pending.as_ref() != Some(&next) => {
                    let mut project = (*gesture.project).clone();
                    if let Some(shot) = project.shots.iter_mut().find(|shot| shot.id == next.id) {
                        *shot = next.clone();
                    }
                    self.camera_interaction.project = Some(Arc::new(project));
                    Arc::make_mut(&mut self.view).camera_workspace.draft =
                        Some(Arc::new(next.clone()));
                    gesture.pending = Some(next);
                    changed = true;
                }
                Err(error) => Arc::make_mut(&mut self.view).error = Some(error.to_string()),
                _ => {}
            }
        }
        self.camera_interaction.rotation = Some(gesture);
        if changed {
            self.publish_editor();
        }
        true
    }

    /// Entering an authored tool previews the saved camera, temporarily suspending inspection.
    fn begin_rotation(
        &mut self,
        handle: CameraRotationHandle,
        down: [f32; 2],
    ) -> Result<(), String> {
        if !self.camera_navigation_available()
            || self.view.workspace.viewport_tool != ViewportTool::Rotate
        {
            return Err("Choose Rotate in the replay viewer before editing a camera.".into());
        }
        if self
            .view
            .workspace
            .timeline
            .locked
            .contains(&recording_studio::tracks::Track::Camera)
        {
            return Err("Unlock the Camera track before rotating a shot.".into());
        }
        let preview = self.camera_preview()?;
        if !preview.interactive || handle.at != self.transport.position() {
            return Err("Wait for the current native replay frame before rotating.".into());
        }
        let project = Arc::clone(self.project()?);
        let shot = project
            .shots
            .iter()
            .find(|shot| shot.id == handle.shot)
            .cloned()
            .ok_or("This camera shot was removed.")?;
        let pose = shot
            .rotation_pose(handle.at)
            .map_err(|error| error.to_string())?;
        let drag = CameraRotationDrag::new(pose, handle.axis, handle.bounds, down)
            .ok_or("This rotation handle has invalid image geometry.")?;
        let inspection = self.view.camera_workspace.view;
        self.release_camera();
        self.cancel_camera_value();
        self.cancel_clip_time();
        self.cancel_title_text();
        self.transport.pause();
        let view = Arc::make_mut(&mut self.view);
        view.selected = Some(handle.shot);
        view.camera_workspace.focused = true;
        view.camera_workspace.looking = false;
        view.camera_workspace.view = None;
        view.camera_workspace.rotation = Some(handle);
        self.camera_interaction.rotation = Some(RotationGesture {
            project,
            shot,
            handle,
            epoch: preview.epoch,
            source: preview.source,
            down,
            last: down,
            value: handle.axis.value(pose),
            drag,
            pending: None,
            inspection,
        });
        self.publish_editor();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
