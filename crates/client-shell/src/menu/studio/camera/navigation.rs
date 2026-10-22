//! View framing and timing controls keep temporary inspection separate from saved camera edits.

use super::*;
use launcher::menu::studio::{CameraNavigationAction as N, NavigationGesture};
use recording_studio::camera::{CameraPose, PathContinuity};

#[derive(Clone, Copy)]
pub(super) struct Pan {
    pose: CameraPose,
    pivot: [f64; 3],
    down: [f32; 2],
    bounds: [i32; 4],
}

#[cfg(test)]
mod tests;

impl Studio {
    /// Settings are runtime inspection preferences; they never enter project history or replay clocks.
    pub(in crate::menu::studio) fn camera_navigation_action(
        &mut self,
        action: N,
    ) -> Result<(), String> {
        if action == N::Level {
            let pose = self
                .view
                .camera_workspace
                .view
                .map(|view| view.pose)
                .or_else(|| {
                    self.displayed_camera_preview()
                        .ok()
                        .map(|preview| preview.render_pose)
                })
                .ok_or("Open a 3D replay view before leveling the horizon.")?;
            if !self.admit_navigation_context() {
                return Err("The replay view changed. Focus it again.".into());
            }
            return self.install_inspection_view(pose.leveled());
        }
        let settings = self.view.camera_workspace.navigation;
        let settings = match action {
            N::Speed(direction) => settings
                .adjusted(f64::from(direction) * 4.0)
                .unwrap_or(settings),
            N::ResetSpeed => settings.reset_speed(),
            N::ToggleSmooth => settings.with_smooth(!settings.smooth),
            N::Level => unreachable!(),
        };
        self.camera_interaction.motion.reset();
        Arc::make_mut(&mut self.view).camera_workspace.navigation = settings;
        Ok(())
    }

    /// A view refresh keeps momentum; a changed document, take or seek invalidates its entire gesture.
    pub(super) fn admit_navigation_context(&mut self) -> bool {
        let context = self.editor.as_ref().and_then(|editor| {
            let epoch = [self.document, editor.revision(), self.transport.epoch()];
            self.view
                .preview
                .camera
                .as_ref()
                .filter(|preview| preview.interactive && preview.epoch == epoch)
                .map(|preview| (epoch, preview.source))
        });
        let Some(context) = context else {
            self.release_camera();
            return false;
        };
        if self
            .camera_interaction
            .navigation_context
            .is_some_and(|previous| previous != context)
        {
            self.release_camera();
            return false;
        }
        self.camera_interaction.navigation_context = Some(context);
        true
    }

    /// The image edge supplies ownership; this handler never consumes timeline or panel scrolling.
    pub(in crate::menu) fn camera_wheel(
        &mut self,
        vertical: f32,
        pixels: bool,
        gesture: NavigationGesture,
    ) -> bool {
        if !vertical.is_finite() || vertical == 0.0 {
            return false;
        }
        if self.navigation_blocked() {
            return true;
        }
        if self.focus_camera().is_err() || !self.admit_navigation_context() {
            return false;
        }
        let steps = f64::from(vertical)
            / if pixels {
                recording_studio::camera::navigation::WHEEL_PIXELS_PER_STEP
            } else {
                1.0
            };
        let workspace = &self.view.camera_workspace;
        let Some(current) = workspace.view else {
            return false;
        };
        if gesture == NavigationGesture::Speed {
            if let Some(settings) = workspace.navigation.adjusted(steps) {
                Arc::make_mut(&mut self.view).camera_workspace.navigation = settings;
                self.camera_interaction.motion.reset();
            }
            return true;
        }
        let next = match gesture {
            NavigationGesture::Zoom => current.pose.zoomed(steps),
            NavigationGesture::Dolly => current
                .pose
                .dollied(steps * workspace.navigation.speed() * 0.125),
            _ => None,
        };
        if let Some(pose) = next.filter(|pose| *pose != current.pose) {
            self.camera_interaction.motion.reset();
            self.publish_inspection_pose(pose);
        }
        true
    }

    /// A captured middle drag uses one frozen view plane and remains valid outside the image until release.
    pub(in crate::menu) fn pan_camera(
        &mut self,
        point: Option<[f32; 2]>,
        held: bool,
        pressed: bool,
        bounds: Option<[i32; 4]>,
    ) -> bool {
        if pressed {
            let Some((point, bounds)) = point.zip(bounds).filter(|(point, bounds)| {
                point.iter().all(|v| v.is_finite())
                    && bounds[2] > bounds[0]
                    && bounds[3] > bounds[1]
            }) else {
                return false;
            };
            let Ok(preview) = self.displayed_camera_preview() else {
                return false;
            };
            if !preview.interactive {
                return false;
            }
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
                .filter(|pivot| {
                    recording_studio::camera::CameraProjection::new(pose, 1.0)
                        .and_then(|projection| projection.project(*pivot))
                        .is_some()
                })
                .or_else(|| {
                    pose.orbit_anchor(recording_studio::camera::navigation::DEFAULT_ORBIT_DISTANCE)
                });
            let Some(pivot) = pivot else {
                return false;
            };
            self.transport.pause();
            self.inspect_camera_pose(pose);
            if !self.admit_navigation_context() {
                return false;
            }
            self.camera_interaction.pan = Some(Pan {
                pose,
                pivot,
                down: point,
                bounds,
            });
        }
        let Some(pan) = self.camera_interaction.pan else {
            return false;
        };
        if !held {
            self.camera_interaction.pan = None;
            return true;
        }
        if !self.admit_navigation_context() {
            return false;
        }
        if let Some(point) = point.filter(|point| point.iter().all(|v| v.is_finite())) {
            let width = f64::from(pan.bounds[2]) - f64::from(pan.bounds[0]);
            let height = f64::from(pan.bounds[3]) - f64::from(pan.bounds[1]);
            let delta = [
                f64::from(point[0] - pan.down[0]) / width,
                f64::from(point[1] - pan.down[1]) / height,
            ];
            if let Some(pose) = pan
                .pose
                .panned(pan.pivot, width / height, delta)
                .filter(|pose| {
                    self.view
                        .camera_workspace
                        .view
                        .is_none_or(|view| view.pose != *pose)
                })
            {
                self.publish_inspection_pose(pose);
            }
        }
        true
    }

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
