//! Camera controls commit ordinary undoable metadata, seeded only by an admitted replay preview.

use recording_studio::{
    camera::{CameraParameter, CameraRigKind, CameraShot},
    preview::CameraPreview,
};

use super::*;

mod values;

pub(super) struct CameraValueEdit {
    document: u64,
    revision: u64,
    shot: Id,
    at: MediaTime,
    parameter: CameraParameter,
}

impl Studio {
    /// Bounded focus fallback exposes the same camera intents as the painted inspector.
    pub(super) fn camera_focus_actions(&self) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.view.preview.camera.is_some() {
            actions.extend([Action::CycleCameraTarget(-1), Action::CycleCameraTarget(1)]);
            actions.extend(CameraRigKind::ALL.iter().copied().map(Action::CreateCamera));
        }
        if let Ok(shot) = self.camera_shot() {
            actions.extend([
                Action::ToggleCameraFine,
                Action::TrimCameraStart,
                Action::TrimCameraEnd,
            ]);
            if matches!(
                shot.kind(),
                CameraRigKind::Player | CameraRigKind::Follow | CameraRigKind::Orbit
            ) {
                actions.push(Action::RetargetCamera);
            }
            if shot.kind() == CameraRigKind::Path {
                actions.extend([
                    Action::AddCameraKey,
                    Action::DeleteCameraKey,
                    Action::CycleCameraEasing,
                    Action::ToggleCameraSmoothing,
                    Action::StepCameraKey(-1),
                    Action::StepCameraKey(1),
                    Action::MoveCameraKey(-1),
                    Action::MoveCameraKey(1),
                ]);
            }
            if self.view.camera_draft.is_some() {
                actions.extend([Action::ApplyCameraValue, Action::CancelCameraValue]);
            }
            for &parameter in shot.parameters() {
                actions.push(Action::EditCameraValue(parameter));
                actions.extend([-1, 1].map(|direction| Action::AdjustCamera {
                    parameter,
                    direction,
                }));
            }
        }
        actions
    }

    /// Rejects a stale displayed pose after document/seek/edit changes instead of keying the wrong frame.
    fn camera_preview(&self) -> Result<CameraPreview, String> {
        let editor = self.editor.as_ref().ok_or("Open a project first.")?;
        self.view
            .preview
            .camera
            .as_ref()
            .filter(|camera| {
                self.view.preview.ready
                    && camera.epoch == [self.document, editor.revision(), self.transport.epoch()]
            })
            .cloned()
            .ok_or_else(|| "Wait for the 3D preview to finish before using its camera pose.".into())
    }

    /// A selected shot takes inspector precedence; otherwise the active shot uses the timeline's same order.
    fn camera_shot(&self) -> Result<CameraShot, String> {
        let project = self.project()?;
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
            .cloned()
            .ok_or_else(|| "Select a camera shot on the timeline first.".into())
    }

    /// Selecting a camera/effect/title clip seeks into its range when the current playhead is outside it.
    pub(super) fn select_timeline_item(&mut self, id: Id) -> Result<(), String> {
        if self.view.selected != Some(id) {
            self.cancel_title_text();
            self.cancel_camera_value();
            Arc::make_mut(&mut self.view).speed_key = None;
        }
        let project = self.project()?;
        let span = project
            .shots
            .iter()
            .find(|shot| shot.id == id)
            .map(|shot| shot.span)
            .or_else(|| {
                project
                    .effects
                    .iter()
                    .find(|effect| effect.id == id)
                    .map(|effect| effect.span)
            })
            .or_else(|| {
                project
                    .titles
                    .iter()
                    .find(|title| title.id == id)
                    .map(|title| title.span)
            });
        if let Some(span) = span {
            let duration = project.duration();
            let start = span.start;
            let contains = span.contains(self.transport.position());
            self.transport.pause();
            if !contains {
                self.transport.seek(start, duration);
            }
        }
        Arc::make_mut(&mut self.view).selected = Some(id);
        Ok(())
    }

    /// A new shot is bounded by the displayed replay clip; no live camera or world is sampled.
    pub(super) fn create_camera(&mut self, kind: CameraRigKind) -> Result<(), String> {
        let preview = self.camera_preview()?;
        let project = self.project()?;
        let at = self.view.preview.at;
        let clip_end = project
            .clips
            .iter()
            .filter(|clip| clip.enabled && clip.source == preview.source)
            .filter_map(|clip| clip.span().ok())
            .filter(|span| span.contains(at))
            .map(|span| span.end)
            .max()
            .ok_or("The displayed replay is outside its timeline clip.")?;
        let end = MediaTime(clip_end.0.min(at.0.saturating_add(6_000_000)));
        let span = Span::new(at, end).map_err(|error| error.to_string())?;
        let target = self.view.camera_target.and_then(|reference| {
            preview
                .targets
                .iter()
                .find(|target| target.matches(reference))
        });
        let id = project.next_id().map_err(|error| error.to_string())?;
        let shot = CameraShot::from_preview(id, span, kind, preview.pose, target)
            .map_err(|error| error.to_string())?;
        let duration = project.duration();
        self.transport.pause();
        self.transport.seek(at, duration);
        self.edit("Add camera shot", &[Command::AddShot { shot }])?;
        Arc::make_mut(&mut self.view).selected = Some(id);
        Ok(())
    }

    /// Target browsing changes only the inspector selection; retargeting an authored shot is explicit.
    pub(super) fn cycle_camera_target(&mut self, direction: i8) {
        let Some(preview) = &self.view.preview.camera else {
            return;
        };
        if preview.targets.is_empty() {
            return;
        }
        let old = self.view.camera_target.and_then(|reference| {
            preview
                .targets
                .iter()
                .position(|target| target.matches(reference))
        });
        let next = match old {
            Some(index) if direction < 0 => {
                (index + preview.targets.len() - 1) % preview.targets.len()
            }
            Some(index) => (index + 1) % preview.targets.len(),
            None => 0,
        };
        let target = preview.targets[next].reference();
        Arc::make_mut(&mut self.view).camera_target = Some(target);
    }

    /// Rebinding preserves follow/orbit parameters and records the selected actor's spawn lifetime.
    pub(super) fn retarget_camera(&mut self) -> Result<(), String> {
        let preview = self.camera_preview()?;
        let reference = self
            .view
            .camera_target
            .ok_or("Choose a recorded target first.")?;
        let target = preview
            .targets
            .iter()
            .find(|target| target.matches(reference))
            .ok_or("This camera target is no longer present at the playhead.")?;
        let shot = self
            .camera_shot()?
            .retargeted(target)
            .map_err(|error| error.to_string())?;
        self.transport.pause();
        self.edit("Retarget camera", &[Command::ReplaceShot { shot }])
    }

    /// Fine adjustment changes step size only; both paths validate and replace a private shot copy.
    pub(super) fn adjust_camera(
        &mut self,
        parameter: CameraParameter,
        direction: i8,
    ) -> Result<(), String> {
        let steps = f64::from(direction.signum()) * if self.view.camera_fine { 0.1 } else { 1.0 };
        let shot = self
            .camera_shot()?
            .adjusted(self.transport.position(), parameter, steps)
            .map_err(|error| error.to_string())?;
        self.transport.pause();
        self.edit("Adjust camera", &[Command::ReplaceShot { shot }])
    }

    /// The key captures the current unmodified rig pose, leaving transient effects separate.
    pub(super) fn add_camera_key(&mut self) -> Result<(), String> {
        let preview = self.camera_preview()?;
        let shot = self
            .camera_shot()?
            .keyed(self.view.preview.at, preview.pose)
            .map_err(|error| error.to_string())?;
        self.transport.pause();
        self.edit("Add camera key", &[Command::ReplaceShot { shot }])
    }

    /// Deletion requires an exact playhead key and leaves at least one path pose.
    pub(super) fn delete_camera_key(&mut self) -> Result<(), String> {
        let shot = self
            .camera_shot()?
            .without_key(self.transport.position())
            .map_err(|error| error.to_string())?;
        self.transport.pause();
        self.edit("Delete camera key", &[Command::ReplaceShot { shot }])
    }

    /// A key's easing controls the segment arriving at that key.
    pub(super) fn cycle_camera_easing(&mut self) -> Result<(), String> {
        let shot = self.camera_shot()?;
        let at = self.transport.position();
        let key = shot
            .key_at(at)
            .ok_or("Select a path key before changing easing.")?;
        let shot = shot
            .with_key_easing(at, key.easing.next())
            .map_err(|error| error.to_string())?;
        self.transport.pause();
        self.edit("Change camera easing", &[Command::ReplaceShot { shot }])
    }

    /// A rail's smoothing choice changes its saved position sampler, not its target or key timings.
    pub(super) fn toggle_camera_smoothing(&mut self) -> Result<(), String> {
        let shot = self.camera_shot()?;
        let recording_studio::camera::CameraMode::Path {
            smooth_position, ..
        } = &shot.mode
        else {
            return Err("Select a path camera first.".into());
        };
        let shot = shot
            .with_path_smoothing(!*smooth_position)
            .map_err(|error| error.to_string())?;
        self.transport.pause();
        self.edit("Change camera smoothing", &[Command::ReplaceShot { shot }])
    }

    /// Key browsing visits actual stored media times, including the legal final boundary key.
    pub(super) fn step_camera_key(&mut self, direction: i8) -> Result<(), String> {
        let shot = self.camera_shot()?;
        let at = self.transport.position();
        let times = shot
            .path_keys()
            .iter()
            .map(|key| {
                shot.span
                    .start
                    .checked_add(key.at)
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let destination = if direction < 0 {
            times.into_iter().rev().find(|time| *time < at)
        } else {
            times.into_iter().find(|time| *time > at)
        }
        .ok_or("There is no further camera key in that direction.")?;
        self.select_camera_key(shot.id, destination)
    }

    /// Key selection shares the timeline's shot identity and clears unrelated uncommitted drafts.
    pub(super) fn select_camera_key(&mut self, id: Id, at: MediaTime) -> Result<(), String> {
        let project = self.project()?;
        let shot = project
            .shots
            .iter()
            .find(|shot| shot.id == id)
            .ok_or("This camera shot is no longer present.")?;
        shot.key_at(at)
            .ok_or("This camera key is no longer present.")?;
        let duration = project.duration();
        self.cancel_camera_value();
        self.cancel_title_text();
        self.transport.pause();
        self.transport.seek(at, duration);
        Arc::make_mut(&mut self.view).selected = Some(id);
        Ok(())
    }

    /// Key nudging uses the project's rational output frame cadence and rejects occupied timestamps.
    pub(super) fn move_camera_key(&mut self, direction: i8) -> Result<(), String> {
        let shot = self.camera_shot()?;
        let at = self.transport.position();
        let project = self.project()?;
        let frame = project
            .export
            .fps
            .frame_at(at)
            .map_err(|error| error.to_string())?;
        let frame = frame
            .checked_add_signed(i64::from(direction.signum()))
            .ok_or("The camera key cannot move before the timeline starts.")?;
        let destination = project
            .export
            .fps
            .frame_time(frame)
            .map_err(|error| error.to_string())?;
        let duration = project.duration();
        let shot = shot
            .with_key_time(at, destination)
            .map_err(|error| error.to_string())?;
        self.cancel_camera_value();
        self.transport.pause();
        self.edit("Move camera key", &[Command::ReplaceShot { shot }])?;
        self.transport.seek(destination, duration);
        Ok(())
    }

    /// Trimming retains path poses and orbit phase at both new boundaries.
    pub(super) fn trim_camera(&mut self, start: bool) -> Result<(), String> {
        let shot = self.camera_shot()?;
        let at = self.transport.position();
        let span = if start {
            Span::new(at, shot.span.end)
        } else {
            Span::new(shot.span.start, at)
        }
        .map_err(|error| error.to_string())?;
        let shot = shot.trimmed(span).map_err(|error| error.to_string())?;
        self.transport.pause();
        self.edit("Trim camera shot", &[Command::ReplaceShot { shot }])
    }
}
