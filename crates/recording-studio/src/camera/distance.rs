//! Arc-length lookup tables are compiled once per immutable timeline snapshot, never on its frames.

use super::*;

const ARC_STEPS: usize = 64;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathTiming {
    #[default]
    Keyed,
    Distance,
}

impl PathTiming {
    /// Legacy documents omit the default and retain their original parameterized path motion.
    pub const fn is_keyed(&self) -> bool {
        matches!(self, Self::Keyed)
    }
}

pub struct CompiledCameraPath {
    start: MediaTime,
    end: MediaTime,
    segments: Vec<ArcSegment>,
}

struct ArcSegment {
    from: CameraKey,
    to: CameraKey,
    controls: [DVec3; 4],
    lengths: [f64; ARC_STEPS + 1],
}

impl CompiledCameraPath {
    /// Only distance-timed rails allocate tables; the owning timeline retains this cache until edited.
    pub fn new(shot: &CameraShot) -> Option<Self> {
        let CameraMode::Path {
            keys,
            smooth_position,
        } = &shot.mode
        else {
            return None;
        };
        if shot.path_timing != PathTiming::Distance || keys.len() < 2 {
            return None;
        }
        Some(Self {
            start: shot.span.start,
            end: shot.span.end,
            segments: (0..keys.len() - 1)
                .map(|index| ArcSegment::new(keys, index, *smooth_position))
                .collect(),
        })
    }

    /// Orientation and lens follow their authored easing while position advances by segment distance.
    pub fn sample(&self, at: MediaTime) -> Option<CameraPose> {
        if at < self.start || at > self.end {
            return None;
        }
        let elapsed = at.saturating_sub(self.start);
        let index = self
            .segments
            .partition_point(|segment| segment.to.at <= elapsed);
        let segment = self.segments.get(index).or_else(|| self.segments.last())?;
        Some(segment.sample(elapsed))
    }
}

impl ArcSegment {
    /// Geometric subdivision uses bounded stack storage; preparing a rail is proportional to key count.
    fn new(keys: &[CameraKey], index: usize, smooth: bool) -> Self {
        let [first, last] = super::path::segment_controls(keys, index, smooth);
        let controls = [
            DVec3::from_array(keys[index].pose.position),
            DVec3::from_array(first),
            DVec3::from_array(last),
            DVec3::from_array(keys[index + 1].pose.position),
        ];
        let mut lengths = [0.0; ARC_STEPS + 1];
        let mut previous = controls[0];
        for step in 1..=ARC_STEPS {
            let point = cubic(controls, step as f64 / ARC_STEPS as f64);
            lengths[step] = lengths[step - 1] + point.distance(previous);
            previous = point;
        }
        Self {
            from: keys[index].clone(),
            to: keys[index + 1].clone(),
            controls,
            lengths,
        }
    }

    /// Degenerate and stationary segments keep exact endpoints and cannot divide by zero.
    fn sample(&self, elapsed: MediaTime) -> CameraPose {
        let amount = (elapsed.0.saturating_sub(self.from.at.0) as f64
            / (self.to.at.0 - self.from.at.0) as f64)
            .clamp(0.0, 1.0);
        let mut pose = self
            .from
            .pose
            .interpolated(self.to.pose, self.to.easing.sample(amount));
        if amount == 0.0 {
            pose.position = self.from.pose.position;
        } else if amount == 1.0 {
            pose.position = self.to.pose.position;
        } else {
            let total = self.lengths[ARC_STEPS];
            let distance = amount * total;
            let upper = self
                .lengths
                .partition_point(|length| *length < distance)
                .clamp(1, ARC_STEPS);
            let before = self.lengths[upper - 1];
            let length = self.lengths[upper] - before;
            let fraction = if length > f64::EPSILON {
                (distance - before) / length
            } else {
                0.0
            };
            let parameter = ((upper - 1) as f64 + fraction) / ARC_STEPS as f64;
            pose.position = cubic(self.controls, parameter).to_array();
        }
        pose
    }
}

/// Admission rejects curves whose finite endpoints still overflow their derived travel geometry.
pub(super) fn validate(keys: &[CameraKey], smooth: bool) -> Result<(), StudioError> {
    for index in 0..keys.len().saturating_sub(1) {
        let segment = ArcSegment::new(keys, index, smooth);
        if !segment.controls.iter().all(|point| point.is_finite())
            || !segment.lengths.iter().all(|length| length.is_finite())
        {
            return Err(StudioError::Invalid(
                "camera path cannot form finite travel distances",
            ));
        }
    }
    Ok(())
}

/// Isolated shot sampling builds only its active segment; frame adapters use the compiled cache above.
pub(super) fn sample(keys: &[CameraKey], elapsed: MediaTime, smooth: bool) -> Option<CameraPose> {
    let index = keys.partition_point(|key| key.at <= elapsed);
    if index == 0 {
        return keys.first().map(|key| key.pose);
    }
    if index == keys.len() {
        return keys.last().map(|key| key.pose);
    }
    Some(ArcSegment::new(keys, index - 1, smooth).sample(elapsed))
}

/// De Casteljau evaluation remains finite for admitted controls and stable at both boundaries.
fn cubic(points: [DVec3; 4], at: f64) -> DVec3 {
    let a = points[0].lerp(points[1], at);
    let b = points[1].lerp(points[2], at);
    let c = points[2].lerp(points[3], at);
    a.lerp(b, at).lerp(b.lerp(c, at), at)
}

impl CameraShot {
    /// Spatial distance mode preserves key times and orientation/lens easing; speeds can differ at keys.
    pub fn with_path_timing(&self, timing: PathTiming) -> Result<Self, StudioError> {
        if !matches!(self.mode, CameraMode::Path { .. }) {
            return Err(StudioError::Invalid(
                "select a path camera before changing travel timing",
            ));
        }
        let mut next = self.clone();
        next.path_timing = timing;
        next.validate()?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests;
