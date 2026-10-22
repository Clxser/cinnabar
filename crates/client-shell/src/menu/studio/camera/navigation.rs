//! View framing and timing controls keep temporary inspection separate from saved camera edits.

use super::*;
use recording_studio::camera::{CameraPose, PathContinuity};

#[cfg(test)]
mod tests;

impl Studio {
    /// Frames the selected recorded subject using the current lens and viewing direction.
    pub(in crate::menu::studio) fn frame_camera_subject(&mut self) -> Result<(), String> {
        let preview = self.camera_preview()?;
        if !preview.interactive {
            return Err("Subject framing requires one native replay view.".into());
        }
        let reference = self
            .view
            .camera_target
            .ok_or("Choose a recorded player first.")?;
        let target = preview
            .targets
            .iter()
            .find(|target| target.matches(reference))
            .ok_or("The selected player is not present at this frame.")?;
        let pose = self
            .view
            .camera_workspace
            .view
            .map_or(preview.render_pose, |view| view.pose);
        let center: [f64; 3] = std::array::from_fn(|axis| {
            target.pose.position[axis] + target.pose.eye_offset.unwrap_or([0.0; 3])[axis] * 0.5
        });
        let next = pose
            .framed(center, 1.35)
            .ok_or("The camera view cannot frame this player.")?;
        self.install_inspection_view(next)
    }

    /// Publishing inspection state changes only the view revision, not the replay seek epoch.
    pub(super) fn install_inspection_view(&mut self, pose: CameraPose) -> Result<(), String> {
        pose.validate().map_err(|error| error.to_string())?;
        self.transport.pause();
        self.inspect_camera_pose(pose);
        Ok(())
    }

    /// Travel timing is a saved undoable shot edit; changing it cannot alter the authored key times.
    pub(in crate::menu::studio) fn toggle_camera_distance_timing(&mut self) -> Result<(), String> {
        let shot = self.camera_shot()?;
        let timing = shot.path_timing.next();
        let shot = shot
            .with_path_timing(timing)
            .map_err(|error| error.to_string())?;
        self.transport.pause();
        self.edit(
            "Change camera travel timing",
            &[Command::ReplaceShot { shot }],
        )
    }

    /// The global travel profile is ordinary saved metadata with the same easing choices as lens keys.
    pub(in crate::menu::studio) fn cycle_camera_global_easing(&mut self) -> Result<(), String> {
        let shot = self.camera_shot()?;
        let shot = shot
            .with_path_global_easing(shot.path_global_easing.next())
            .map_err(|error| error.to_string())?;
        self.transport.pause();
        self.edit(
            "Change whole rail travel easing",
            &[Command::ReplaceShot { shot }],
        )
    }

    /// Aligned handles are an explicit undoable geometry choice, independent of travel and lens timing.
    pub(in crate::menu::studio) fn toggle_camera_continuity(&mut self) -> Result<(), String> {
        let shot = self.camera_shot()?;
        let continuity = match shot.path_continuity {
            PathContinuity::Authored => PathContinuity::Smooth,
            PathContinuity::Smooth => PathContinuity::Authored,
        };
        let shot = shot
            .with_path_continuity(continuity)
            .map_err(|error| error.to_string())?;
        self.transport.pause();
        self.edit(
            "Change camera tangent continuity",
            &[Command::ReplaceShot { shot }],
        )
    }
}
