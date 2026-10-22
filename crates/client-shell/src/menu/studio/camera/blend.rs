//! Entry transition edits use the normal project transaction and camera-track lock policy.

use super::*;
use launcher::menu::studio::CameraBlendAction;
use recording_studio::camera::CameraBlend;

impl Studio {
    /// Fine duration steps use one output frame; coarse steps use the domain-owned quarter second.
    pub(in crate::menu::studio) fn camera_blend_action(
        &mut self,
        action: CameraBlendAction,
    ) -> Result<(), String> {
        if self
            .view
            .workspace
            .timeline
            .locked
            .contains(&recording_studio::tracks::Track::Camera)
        {
            return Err("Unlock the camera track before changing its transition.".into());
        }
        if action == CameraBlendAction::Duration(0) {
            return Ok(());
        }
        let shot = self.camera_shot()?;
        let blend = match action {
            CameraBlendAction::Toggle => {
                if shot.blend_in.is_some() {
                    None
                } else {
                    Some(CameraBlend {
                        duration: CameraBlend::DEFAULT_DURATION.min(shot.span.duration()),
                        easing: recording_studio::effect::Easing::Smooth,
                    })
                }
            }
            CameraBlendAction::Duration(direction) => {
                let mut blend = shot
                    .blend_in
                    .ok_or("Enable an entry blend before changing its duration.")?;
                let step = if self.view.camera_fine {
                    self.project()?
                        .export
                        .fps
                        .frame_time(1)
                        .map_err(|error| error.to_string())?
                } else {
                    CameraBlend::DURATION_STEP
                };
                blend.duration = if direction > 0 {
                    MediaTime(blend.duration.0.saturating_add(step.0)).min(shot.span.duration())
                } else {
                    blend.duration.saturating_sub(step).max(MediaTime(1))
                };
                Some(blend)
            }
            CameraBlendAction::CycleEasing => {
                let mut blend = shot
                    .blend_in
                    .ok_or("Enable an entry blend before changing its easing.")?;
                blend.easing = blend.easing.next();
                Some(blend)
            }
        };
        let next = shot
            .with_blend_in(blend)
            .map_err(|error| error.to_string())?;
        if next == shot {
            return Ok(());
        }
        self.transport.pause();
        self.release_camera();
        self.edit(
            "Change camera entry transition",
            &[Command::ReplaceShot { shot: next }],
        )?;
        self.reset_camera_view();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
