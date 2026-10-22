//! Stateless camera authoring shared by native controls and future structured editing clients.

use super::*;

macro_rules! rigs {
    ($($kind:ident => $label:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum CameraRigKind { $($kind),+ }
        impl CameraRigKind {
            pub const ALL: &[Self] = &[$(Self::$kind),+];
            pub const fn label(self) -> &'static str { match self { $(Self::$kind => $label),+ } }
        }
    };
}
rigs! { Player => "Become entity", Free => "Free", Path => "Path", Track => "Track entity", Follow => "Follow", Orbit => "Orbit" }

macro_rules! parameters {
    ($($kind:ident => ($label:literal, $step:literal)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum CameraParameter { $($kind),+ }
        impl CameraParameter {
            pub const fn label(self) -> &'static str { match self { $(Self::$kind => $label),+ } }
            pub const fn step(self) -> f64 { match self { $(Self::$kind => $step),+ } }
        }
    };
}
parameters! {
    X => ("X", 0.5), Y => ("Y", 0.5), Z => ("Z", 0.5),
    Yaw => ("Yaw", 5.0), Pitch => ("Pitch", 5.0), Roll => ("Roll", 5.0), Fov => ("FOV", 5.0),
    Radius => ("Radius", 0.5), Height => ("Height", 0.5), StartAngle => ("Start angle", 5.0), Turns => ("Turns", 0.05), LookAhead => ("Look ahead", 0.05),
}

impl CameraShot {
    /// Creates an editable rig from the displayed source's actual pose and admitted target identity.
    pub fn from_preview(
        id: Id,
        span: Span,
        kind: CameraRigKind,
        pose: CameraPose,
        target: Option<&CameraTarget>,
    ) -> Result<Self, StudioError> {
        use CameraRigKind as K;
        let reference = || {
            target
                .map(CameraTarget::reference)
                .ok_or(StudioError::Invalid(
                    "choose a recorded camera target first",
                ))
        };
        let mode = match kind {
            K::Free => CameraMode::Free { pose },
            K::Path => CameraMode::Path {
                keys: vec![CameraKey {
                    at: MediaTime::ZERO,
                    pose,
                    easing: Easing::Smooth,
                    handles: None,
                }],
                smooth_position: true,
            },
            K::Player => {
                if target.and_then(|target| target.pose.eye_offset).is_none() {
                    return Err(StudioError::Invalid(
                        "this target has no recorded native eye position",
                    ));
                }
                CameraMode::Player {
                    actor: reference()?.runtime_id,
                    fov: pose.fov,
                }
            }
            K::Track => CameraMode::Track {
                actor: reference()?.runtime_id,
                pose,
            },
            K::Follow | K::Orbit => {
                let target =
                    target.ok_or(StudioError::Invalid("choose a recorded subject first"))?;
                let delta =
                    DVec3::from_array(pose.position) - DVec3::from_array(target.pose.centre());
                if kind == K::Follow {
                    let (sin, cos) = target.pose.yaw.to_radians().sin_cos();
                    CameraMode::Follow {
                        actor: reference()?.runtime_id,
                        offset: [
                            delta.x * cos + delta.z * sin,
                            delta.y,
                            -delta.x * sin + delta.z * cos,
                        ],
                        look_ahead: 0.0,
                        fov: pose.fov,
                        roll: pose.roll,
                    }
                } else {
                    let radius = delta.x.hypot(delta.z);
                    if !(0.1..=1024.0).contains(&radius) || delta.y.abs() > 1024.0 {
                        return Err(StudioError::Invalid(
                            "move the camera 0.1–1024 blocks from the subject before creating an orbit",
                        ));
                    }
                    CameraMode::Orbit {
                        actor: reference()?.runtime_id,
                        radius,
                        height: delta.y,
                        start_degrees: delta.x.atan2(delta.z).to_degrees(),
                        turns: 0.25,
                        fov: pose.fov,
                        roll: pose.roll,
                    }
                }
            }
        };
        let shot = Self {
            id,
            name: format!("{} camera", kind.label()),
            span,
            mode,
            hide_hud: true,
            hide_hand: true,
            target_lifetime: if matches!(kind, K::Free | K::Path) {
                None
            } else {
                Some(reference()?.lifetime.expect("captured target lifetime"))
            },
            fov_curve: None,
            fov_offset: MediaTime::ZERO,
            path_timing: PathTiming::Keyed,
            path_offset: MediaTime::ZERO,
        };
        shot.validate()?;
        Ok(shot)
    }

    /// Existing rigs expose only controls that their complete schema actually owns.
    pub fn kind(&self) -> CameraRigKind {
        match &self.mode {
            CameraMode::Player { actor: _, fov: _ } => CameraRigKind::Player,
            CameraMode::Free { pose: _ } => CameraRigKind::Free,
            CameraMode::Track { .. } => CameraRigKind::Track,
            CameraMode::Path {
                keys: _,
                smooth_position: _,
            } => CameraRigKind::Path,
            CameraMode::Follow {
                actor: _,
                offset: _,
                look_ahead: _,
                fov: _,
                roll: _,
            } => CameraRigKind::Follow,
            CameraMode::Orbit {
                actor: _,
                radius: _,
                height: _,
                start_degrees: _,
                turns: _,
                fov: _,
                roll: _,
            } => CameraRigKind::Orbit,
        }
    }

    /// Parameter lists are exhaustive by rig; adding a mode requires choosing its authoring contract.
    pub fn parameters(&self) -> &'static [CameraParameter] {
        use CameraParameter as P;
        match self.kind() {
            CameraRigKind::Free | CameraRigKind::Path => {
                &[P::X, P::Y, P::Z, P::Yaw, P::Pitch, P::Roll, P::Fov]
            }
            CameraRigKind::Track => &[P::X, P::Y, P::Z, P::Roll, P::Fov],
            CameraRigKind::Follow => &[P::X, P::Y, P::Z, P::LookAhead, P::Roll, P::Fov],
            CameraRigKind::Orbit => &[
                P::Radius,
                P::Height,
                P::StartAngle,
                P::Turns,
                P::Roll,
                P::Fov,
            ],
            CameraRigKind::Player => &[P::Fov],
        }
    }

    /// Resolves position parameters to a path's sampled pose without selecting an arbitrary neighbouring key.
    pub fn parameter(&self, parameter: CameraParameter, at: MediaTime) -> Option<f64> {
        use CameraParameter as P;
        if parameter == P::Fov
            && let Some(value) = self.fov_override(at)
        {
            return Some(value);
        }
        match &self.mode {
            CameraMode::Free { pose } => pose_value(*pose, parameter),
            CameraMode::Track { pose, .. } => {
                if matches!(parameter, P::Yaw | P::Pitch) {
                    None
                } else {
                    pose_value(*pose, parameter)
                }
            }
            CameraMode::Path { .. } => pose_value(self.path_pose(at)?, parameter),
            CameraMode::Player { actor: _, fov } => (parameter == P::Fov).then_some(*fov),
            CameraMode::Follow {
                actor: _,
                offset,
                look_ahead,
                fov,
                roll,
            } => match parameter {
                P::X => Some(offset[0]),
                P::Y => Some(offset[1]),
                P::Z => Some(offset[2]),
                P::LookAhead => Some(*look_ahead),
                P::Fov => Some(*fov),
                P::Roll => Some(*roll),
                _ => None,
            },
            CameraMode::Orbit {
                actor: _,
                radius,
                height,
                start_degrees,
                turns,
                fov,
                roll,
            } => match parameter {
                P::Radius => Some(*radius),
                P::Height => Some(*height),
                P::StartAngle => Some(*start_degrees),
                P::Turns => Some(*turns),
                P::Fov => Some(*fov),
                P::Roll => Some(*roll),
                _ => None,
            },
        }
    }

    /// Edits a private copy. A path adjustment creates/changes an exact key at the current playhead.
    pub fn adjusted(
        &self,
        at: MediaTime,
        parameter: CameraParameter,
        steps: f64,
    ) -> Result<Self, StudioError> {
        if !steps.is_finite() {
            return Err(StudioError::Invalid(
                "camera adjustment is outside its shot",
            ));
        }
        let current = self.parameter(parameter, at).ok_or(StudioError::Invalid(
            "this rig does not own that camera parameter",
        ))?;
        use CameraParameter as P;
        let value = current + steps * parameter.step();
        let value = match parameter {
            P::Pitch => value.clamp(-90.0, 90.0),
            P::Fov => value.clamp(CAMERA_FOV_RANGE.start + 0.01, CAMERA_FOV_RANGE.end - 0.01),
            P::Radius => value.clamp(0.1, 1024.0),
            P::Height | P::Turns => value.clamp(-1024.0, 1024.0),
            P::LookAhead => value.clamp(0.0, 10.0),
            _ => value,
        };
        self.with_parameter(at, parameter, value)
    }

    /// Assigns a finite numeric value exactly; invalid projection and rig bounds produce an error.
    pub fn with_parameter(
        &self,
        at: MediaTime,
        parameter: CameraParameter,
        value: f64,
    ) -> Result<Self, StudioError> {
        if at < self.span.start || at > self.span.end || !value.is_finite() {
            return Err(StudioError::Invalid(
                "camera value is outside its shot or is not finite",
            ));
        }
        self.parameter(parameter, at).ok_or(StudioError::Invalid(
            "this rig does not own that camera parameter",
        ))?;
        use CameraParameter as P;
        if parameter == P::Fov && self.fov_curve.is_some() {
            return self.with_fov_key(at, value);
        }
        let mut next = self.clone();
        match &mut next.mode {
            CameraMode::Free { pose } | CameraMode::Track { pose, .. } => {
                set_pose(pose, parameter, value)?
            }
            CameraMode::Path {
                keys,
                smooth_position: _,
            } => {
                let offset = next
                    .path_offset
                    .checked_add(at.saturating_sub(next.span.start))?;
                let mut pose = self
                    .path_pose(at)
                    .ok_or(StudioError::Invalid("camera path has no pose"))?;
                set_pose(&mut pose, parameter, value)?;
                set_key(
                    keys,
                    CameraKey {
                        at: offset,
                        pose,
                        easing: Easing::Smooth,
                        handles: None,
                    },
                )?;
            }
            CameraMode::Player { actor: _, fov } => *fov = value,
            CameraMode::Follow {
                actor: _,
                offset,
                look_ahead,
                fov,
                roll,
            } => match parameter {
                P::X => offset[0] = value,
                P::Y => offset[1] = value,
                P::Z => offset[2] = value,
                P::LookAhead => *look_ahead = value,
                P::Fov => *fov = value,
                P::Roll => *roll = value,
                _ => unreachable!("parameter admitted above"),
            },
            CameraMode::Orbit {
                actor: _,
                radius,
                height,
                start_degrees,
                turns,
                fov,
                roll,
            } => match parameter {
                P::Radius => *radius = value,
                P::Height => *height = value,
                P::StartAngle => *start_degrees = value,
                P::Turns => *turns = value,
                P::Fov => *fov = value,
                P::Roll => *roll = value,
                _ => unreachable!("parameter admitted above"),
            },
        }
        next.validate()?;
        Ok(next)
    }

    /// Records the current preview pose as a path key; an existing key keeps its easing choice.
    pub fn keyed(&self, at: MediaTime, pose: CameraPose) -> Result<Self, StudioError> {
        if at < self.span.start || at > self.span.end {
            return Err(StudioError::Invalid("camera key is outside its shot"));
        }
        let mut next = self.clone();
        let CameraMode::Path {
            keys,
            smooth_position: _,
        } = &mut next.mode
        else {
            return Err(StudioError::Invalid(
                "select a path camera before adding a key",
            ));
        };
        set_key(
            keys,
            CameraKey {
                at: self
                    .path_offset
                    .checked_add(at.saturating_sub(self.span.start))?,
                pose,
                easing: Easing::Smooth,
                handles: None,
            },
        )?;
        next.validate()?;
        Ok(next)
    }

    /// Rebinding an actor is explicit and retains all authored rig parameters.
    pub fn retargeted(&self, target: &CameraTarget) -> Result<Self, StudioError> {
        let mut next = self.clone();
        match &mut next.mode {
            CameraMode::Player { actor, fov: _ } => {
                if target.pose.eye_offset.is_none() {
                    return Err(StudioError::Invalid(
                        "this target has no recorded native eye position",
                    ));
                }
                *actor = target.runtime_id;
            }
            CameraMode::Follow {
                actor,
                offset: _,
                look_ahead: _,
                fov: _,
                roll: _,
            }
            | CameraMode::Orbit {
                actor,
                radius: _,
                height: _,
                start_degrees: _,
                turns: _,
                fov: _,
                roll: _,
            }
            | CameraMode::Track { actor, pose: _ } => *actor = target.runtime_id,
            CameraMode::Free { pose: _ }
            | CameraMode::Path {
                keys: _,
                smooth_position: _,
            } => return Err(StudioError::Invalid("this camera has no actor target")),
        }
        next.target_lifetime = target.reference().lifetime;
        next.validate()?;
        Ok(next)
    }

    /// Trimming preserves original rail keys/easing and orbit phase without resampling motion.
    pub fn trimmed(&self, span: Span) -> Result<Self, StudioError> {
        span.validate()?;
        if span.start < self.span.start || span.end > self.span.end {
            return Err(StudioError::Invalid(
                "camera trim must stay inside its shot",
            ));
        }
        if span == self.span {
            return Ok(self.clone());
        }
        let mut next = self.clone();
        let offset = span.start.saturating_sub(self.span.start);
        match &mut next.mode {
            CameraMode::Path { .. } => {
                next.path_offset = next.path_offset.checked_add(offset)?;
            }
            CameraMode::Orbit {
                actor: _,
                radius: _,
                height: _,
                start_degrees,
                turns,
                fov: _,
                roll: _,
            } => {
                *start_degrees += 360.0 * *turns * offset.0 as f64 / self.span.duration().0 as f64;
                *turns *= span.duration().0 as f64 / self.span.duration().0 as f64;
            }
            CameraMode::Player { actor: _, fov: _ }
            | CameraMode::Free { pose: _ }
            | CameraMode::Track { .. }
            | CameraMode::Follow {
                actor: _,
                offset: _,
                look_ahead: _,
                fov: _,
                roll: _,
            } => {}
        }
        if next.fov_curve.is_some() {
            next.fov_offset = next.fov_offset.checked_add(offset)?;
        }
        next.span = span;
        next.validate()?;
        Ok(next)
    }
}

/// Named pose controls share one component mapping across the inspector and saved edits.
fn pose_value(pose: CameraPose, parameter: CameraParameter) -> Option<f64> {
    use CameraParameter as P;
    match parameter {
        P::X => Some(pose.position[0]),
        P::Y => Some(pose.position[1]),
        P::Z => Some(pose.position[2]),
        P::Yaw => Some(pose.yaw),
        P::Pitch => Some(pose.pitch),
        P::Roll => Some(pose.roll),
        P::Fov => Some(pose.fov),
        _ => None,
    }
}

/// Rejects unsupported parameters instead of silently editing a different field.
fn set_pose(
    pose: &mut CameraPose,
    parameter: CameraParameter,
    value: f64,
) -> Result<(), StudioError> {
    use CameraParameter as P;
    match parameter {
        P::X => pose.position[0] = value,
        P::Y => pose.position[1] = value,
        P::Z => pose.position[2] = value,
        P::Yaw => pose.yaw = value,
        P::Pitch => pose.pitch = value,
        P::Roll => pose.roll = value,
        P::Fov => pose.fov = value,
        _ => return Err(StudioError::Invalid("this is not a pose parameter")),
    }
    Ok(())
}

/// Keys stay ordered, bounded and unique; replacing one does not reset its authored interpolation.
fn set_key(keys: &mut Vec<CameraKey>, mut key: CameraKey) -> Result<(), StudioError> {
    match keys.binary_search_by_key(&key.at, |old| old.at) {
        Ok(index) => {
            key.easing = keys[index].easing;
            key.handles = keys[index].handles;
            keys[index] = key;
        }
        Err(index) => {
            limit("camera keys", keys.len().saturating_add(1), MAX_CAMERA_KEYS)?;
            keys.insert(index, key);
        }
    }
    Ok(())
}
