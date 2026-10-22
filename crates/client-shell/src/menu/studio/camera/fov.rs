//! Lens-key actions use the same camera selection, exact playhead and undo journal as pose edits.

use super::*;

impl Studio {
    /// Lens edits never require reconstructing a path or replacing an actor-driven rig.
    pub(in crate::menu::studio) fn camera_fov_action(
        &mut self,
        action: Action,
    ) -> Result<(), String> {
        let shot = self.camera_shot()?;
        let at = self.transport.position();
        let replacement = match action {
            Action::AddCameraFovKey => shot.with_fov_key(
                at,
                shot.parameter(CameraParameter::Fov, at)
                    .ok_or("This camera has no field of view.")?,
            ),
            Action::DeleteCameraFovKey => shot.without_fov_key(at),
            Action::CycleCameraFovEasing => {
                let key = shot
                    .fov_key_at(at)
                    .ok_or("Select an exact FOV key first.")?;
                shot.with_fov_easing(at, key.easing.next())
            }
            Action::ClearCameraFov => {
                let mut shot = shot;
                shot.fov_curve = None;
                shot.fov_offset = MediaTime::ZERO;
                Ok(shot)
            }
            Action::StepCameraFovKey(direction) => {
                let destination = if direction < 0 {
                    shot.fov_keys()
                        .map(|(at, _)| at)
                        .filter(|time| *time < at)
                        .last()
                } else {
                    shot.fov_keys().map(|(at, _)| at).find(|time| *time > at)
                }
                .ok_or("There is no further FOV key in that direction.")?;
                return self.select_camera_fov_key(shot.id, destination);
            }
            _ => return Err("This action does not edit camera FOV keys.".into()),
        }
        .map_err(|error| error.to_string())?;
        self.cancel_camera_value();
        self.transport.pause();
        self.edit(
            "Edit camera FOV",
            &[Command::ReplaceShot { shot: replacement }],
        )
    }

    /// Choosing a lens key changes selection/time without selecting an unrelated pose handle.
    pub(in crate::menu::studio) fn select_camera_fov_key(
        &mut self,
        id: Id,
        at: MediaTime,
    ) -> Result<(), String> {
        let project = self.project()?;
        project
            .shots
            .iter()
            .find(|shot| shot.id == id)
            .and_then(|shot| shot.fov_key_at(at))
            .ok_or("This FOV key is no longer present.")?;
        let duration = project.duration();
        self.cancel_camera_value();
        self.cancel_title_text();
        self.transport.pause();
        self.transport.seek(at, duration);
        let view = Arc::make_mut(&mut self.view);
        view.selected = Some(id);
        view.camera_workspace.key = None;
        Ok(())
    }
}
