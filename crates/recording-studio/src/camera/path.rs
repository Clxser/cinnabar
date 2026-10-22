//! Authored Bezier handles share the same evaluated path as preview, export and projected controls.

use super::*;

const MAX_HANDLE_OFFSET: f64 = 4096.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraHandles {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incoming: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outgoing: Option<[f64; 3]>,
}

impl CameraHandles {
    /// Control offsets are finite and bounded independently of world coordinates.
    pub(super) fn validate(&self, position: [f64; 3]) -> Result<(), StudioError> {
        for offset in [self.incoming, self.outgoing].into_iter().flatten() {
            if offset.iter().zip(position).any(|(&offset, position)| {
                !offset.is_finite()
                    || offset.abs() > MAX_HANDLE_OFFSET
                    || !(position + offset).is_finite()
            }) {
                return Err(StudioError::Invalid(
                    "camera path handle is outside its finite bound",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathControl {
    Key,
    Incoming,
    Outgoing,
}

impl CameraShot {
    /// A bounded geometric trace clips support segments to the visible source-time range.
    pub fn path_geometry_samples<const N: usize>(&self, segment: usize) -> Option<[[f64; 3]; N]> {
        if N < 2 {
            return None;
        }
        if self.path_timing == PathTiming::GlobalDistance {
            return CompiledCameraPath::new(self)?.geometry_samples(segment);
        }
        let CameraMode::Path {
            keys,
            smooth_position,
        } = &self.mode
        else {
            return None;
        };
        let start = keys.get(segment)?;
        let end = keys.get(segment.checked_add(1)?)?;
        let low = start.at.max(self.path_offset);
        let high = end
            .at
            .min(self.path_offset.checked_add(self.span.duration()).ok()?);
        if high <= low {
            return None;
        }
        let duration = (end.at.0 - start.at.0) as f64;
        let amounts = [
            (low.0 - start.at.0) as f64 / duration,
            (high.0 - start.at.0) as f64 / duration,
        ];
        let [low, high] = if self.path_timing == PathTiming::Distance {
            super::distance::geometry_parameters(
                keys,
                segment,
                *smooth_position,
                self.path_continuity,
                amounts,
            )
        } else {
            amounts.map(|amount| end.easing.sample(amount))
        };
        Some(std::array::from_fn(|index| {
            bezier_position(
                keys,
                segment,
                low + (high - low) * index as f64 / (N - 1) as f64,
                *smooth_position,
                self.path_continuity,
            )
        }))
    }

    /// A geometric rail point avoids arc-table preparation when painting a path overlay.
    pub fn path_geometry_position(&self, segment: usize, amount: f64) -> Option<[f64; 3]> {
        if !amount.is_finite() {
            return None;
        }
        let CameraMode::Path {
            keys,
            smooth_position,
        } = &self.mode
        else {
            return None;
        };
        if segment.checked_add(1)? >= keys.len() {
            return None;
        }
        Some(bezier_position(
            keys,
            segment,
            amount.clamp(0.0, 1.0),
            *smooth_position,
            self.path_continuity,
        ))
    }

    /// Samples a rail at an inclusive shot boundary without querying an actor or live world.
    pub fn path_pose(&self, at: MediaTime) -> Option<CameraPose> {
        if at < self.span.start || at > self.span.end {
            return None;
        }
        let CameraMode::Path {
            keys,
            smooth_position,
        } = &self.mode
        else {
            return None;
        };
        let elapsed = self
            .path_offset
            .checked_add(at.saturating_sub(self.span.start))
            .ok()?;
        match self.path_timing {
            PathTiming::GlobalDistance => CompiledCameraPath::new(self)
                .and_then(|path| path.sample(at))
                .or_else(|| keys.first().map(|key| key.pose)),
            PathTiming::Distance => {
                super::distance::sample(keys, elapsed, *smooth_position, self.path_continuity)
            }
            PathTiming::Keyed => {
                sample_path_policy(keys, elapsed, *smooth_position, self.path_continuity)
            }
        }
    }

    /// Returns the actual control point, including automatic tangents for untouched handles.
    pub fn path_control(&self, at: MediaTime, control: PathControl) -> Option<[f64; 3]> {
        let key = self.key_at(at)?;
        let CameraMode::Path {
            keys,
            smooth_position,
        } = &self.mode
        else {
            return None;
        };
        let index = keys.binary_search_by_key(&key.at, |key| key.at).ok()?;
        match control {
            PathControl::Key => Some(key.pose.position),
            PathControl::Incoming if index > 0 => {
                Some(segment_controls(keys, index - 1, *smooth_position, self.path_continuity)[1])
            }
            PathControl::Outgoing if index + 1 < keys.len() => {
                Some(segment_controls(keys, index, *smooth_position, self.path_continuity)[0])
            }
            _ => None,
        }
    }

    /// Moves one exact control; translating a key retains its authored relative handles.
    pub fn with_path_control(
        &self,
        at: MediaTime,
        control: PathControl,
        position: [f64; 3],
    ) -> Result<Self, StudioError> {
        self.path_control(at, control)
            .ok_or(StudioError::Invalid("select a visible camera path control"))?;
        let offset = self.key_at(at).expect("admitted path key").at;
        let mut next = self.clone();
        let CameraMode::Path { keys, .. } = &mut next.mode else {
            unreachable!()
        };
        let index = keys
            .binary_search_by_key(&offset, |key| key.at)
            .expect("admitted key");
        let key = &mut keys[index];
        match control {
            PathControl::Key => key.pose.position = position,
            PathControl::Incoming | PathControl::Outgoing => {
                let offset = std::array::from_fn(|axis| position[axis] - key.pose.position[axis]);
                let handles = key.handles.get_or_insert_with(CameraHandles::default);
                match control {
                    PathControl::Incoming => handles.incoming = Some(offset),
                    PathControl::Outgoing => handles.outgoing = Some(offset),
                    PathControl::Key => unreachable!(),
                }
            }
        }
        next.validate()?;
        Ok(next)
    }

    /// Restores an exact key's automatic tangents without changing its pose, time or easing.
    pub fn without_path_handles(&self, at: MediaTime) -> Result<Self, StudioError> {
        let offset = self
            .key_at(at)
            .ok_or(StudioError::Invalid("select a camera path key"))?
            .at;
        let mut next = self.clone();
        let CameraMode::Path { keys, .. } = &mut next.mode else {
            unreachable!()
        };
        keys.iter_mut()
            .find(|key| key.at == offset)
            .expect("admitted key")
            .handles = None;
        Ok(next)
    }

    /// Applies an inspected fly pose to a free rig or one path key as a single complete pose edit.
    pub fn with_pose(&self, at: MediaTime, pose: CameraPose) -> Result<Self, StudioError> {
        pose.validate()?;
        if at < self.span.start || at > self.span.end {
            return Err(StudioError::Invalid("camera pose is outside its shot"));
        }
        match &self.mode {
            CameraMode::Path { .. } => self.keyed(at, pose),
            CameraMode::Free { .. } | CameraMode::Track { .. } => {
                let mut next = self.clone();
                match &mut next.mode {
                    CameraMode::Free { pose: current }
                    | CameraMode::Track { pose: current, .. } => *current = pose,
                    _ => unreachable!(),
                }
                next.validate()?;
                Ok(next)
            }
            _ => Err(StudioError::Invalid(
                "apply a fly view to a free camera or path key",
            )),
        }
    }
}

/// Missing handles use the same time-weighted tangents as a legacy smooth rail, or linear thirds.
pub(super) fn segment_controls(
    keys: &[CameraKey],
    index: usize,
    smooth: bool,
    continuity: PathContinuity,
) -> [[f64; 3]; 2] {
    if continuity == PathContinuity::Smooth {
        return super::continuity::controls(keys, index);
    }
    let (a, b) = (&keys[index], &keys[index + 1]);
    let duration = b.at.saturating_sub(a.at).seconds();
    let automatic = std::array::from_fn::<_, 3, _>(|axis| {
        let slope = (b.pose.position[axis] - a.pose.position[axis]) / duration;
        let first = if smooth {
            index.checked_sub(1).map_or(slope, |previous| {
                let previous = &keys[previous];
                let before = a.at.saturating_sub(previous.at).seconds();
                tangent(
                    (a.pose.position[axis] - previous.pose.position[axis]) / before,
                    slope,
                    before,
                    duration,
                )
            })
        } else {
            slope
        };
        let last = if smooth {
            keys.get(index + 2).map_or(slope, |following| {
                let after = following.at.saturating_sub(b.at).seconds();
                tangent(
                    slope,
                    (following.pose.position[axis] - b.pose.position[axis]) / after,
                    duration,
                    after,
                )
            })
        } else {
            slope
        };
        [first * duration / 3.0, -last * duration / 3.0]
    });
    [
        std::array::from_fn(|axis| {
            a.pose.position[axis]
                + a.handles
                    .and_then(|h| h.outgoing)
                    .map_or(automatic[axis][0], |offset| offset[axis])
        }),
        std::array::from_fn(|axis| {
            b.pose.position[axis]
                + b.handles
                    .and_then(|h| h.incoming)
                    .map_or(automatic[axis][1], |offset| offset[axis])
        }),
    ]
}

/// Cubic position uses the arriving key's easing parameter already applied by the path sampler.
pub(super) fn bezier_position(
    keys: &[CameraKey],
    index: usize,
    t: f64,
    smooth: bool,
    continuity: PathContinuity,
) -> [f64; 3] {
    let [a, b] = segment_controls(keys, index, smooth, continuity);
    let start = DVec3::from_array(keys[index].pose.position);
    let end = DVec3::from_array(keys[index + 1].pose.position);
    let left = start.lerp(DVec3::from_array(a), t);
    let middle = DVec3::from_array(a).lerp(DVec3::from_array(b), t);
    let right = DVec3::from_array(b).lerp(end, t);
    left.lerp(middle, t)
        .lerp(middle.lerp(right, t), t)
        .to_array()
}
