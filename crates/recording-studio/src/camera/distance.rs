//! Arc-length lookup tables are compiled once per immutable timeline snapshot, never on its frames.

use super::*;

const ARC_STEPS: usize = 64;

mod global;
mod integration;
use global::GlobalRail;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathTiming {
    #[default]
    Keyed,
    Distance,
    GlobalDistance,
}

impl PathTiming {
    /// Legacy documents omit the default and retain their original parameterized path motion.
    pub const fn is_keyed(&self) -> bool {
        matches!(self, Self::Keyed)
    }

    /// Saved travel choices retain the original keyed and per-interval contracts.
    pub const fn next(self) -> Self {
        match self {
            Self::Keyed => Self::Distance,
            Self::Distance => Self::GlobalDistance,
            Self::GlobalDistance => Self::Keyed,
        }
    }

    /// The same names label domain choices and undoable inspector controls.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Keyed => "Key easing",
            Self::Distance => "Even speed between keys",
            Self::GlobalDistance => "Whole rail distance",
        }
    }
}

/// Global travel starts linear unless the author chooses an acceleration profile.
pub(super) const fn linear_easing() -> Easing {
    Easing::Linear
}

/// Omit the compatibility default from documents without inventing a second easing registry.
pub(super) const fn is_linear(easing: &Easing) -> bool {
    matches!(easing, Easing::Linear)
}

pub struct CompiledCameraPath {
    start: MediaTime,
    end: MediaTime,
    offset: MediaTime,
    segments: Vec<ArcSegment>,
    global: Option<GlobalRail>,
    accuracy: PathAccuracy,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PathAccuracy {
    pub distance: f64,
    /// Difference between independent quadrature rules estimates error; it is not a rigorous bound.
    pub estimated_length_error: f64,
}

struct ArcSegment {
    from: CameraKey,
    to: CameraKey,
    controls: [DVec3; 4],
    lengths: [f64; ARC_STEPS + 1],
    estimated_error: f64,
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
        if shot.path_timing == PathTiming::Keyed || keys.len() < 2 {
            return None;
        }
        let segments: Vec<_> = (0..keys.len() - 1)
            .map(|index| ArcSegment::new(keys, index, *smooth_position, shot.path_continuity))
            .collect();
        let global = (shot.path_timing == PathTiming::GlobalDistance)
            .then(|| GlobalRail::new(&segments, shot.path_global_easing));
        let accuracy = PathAccuracy {
            distance: segments.iter().map(ArcSegment::length).sum(),
            estimated_length_error: segments.iter().map(|segment| segment.estimated_error).sum(),
        };
        Some(Self {
            start: shot.span.start,
            end: shot.span.end,
            offset: shot.path_offset,
            segments,
            global,
            accuracy,
        })
    }

    /// Orientation and lens follow their authored easing while position advances by segment distance.
    pub fn sample(&self, at: MediaTime) -> Option<CameraPose> {
        if at < self.start || at > self.end {
            return None;
        }
        let elapsed = self
            .offset
            .checked_add(at.saturating_sub(self.start))
            .ok()?;
        let index = self
            .segments
            .partition_point(|segment| segment.to.at <= elapsed);
        let segment = self.segments.get(index).or_else(|| self.segments.last())?;
        let mut pose = if self.global.is_some() {
            segment.orientation(elapsed)
        } else {
            segment.sample(elapsed)
        };
        if let Some(global) = &self.global {
            pose.position = global.position(&self.segments, elapsed);
        }
        Some(pose)
    }

    /// Analytic world velocity uses traveled distance and tangent direction, independent of seek history.
    pub fn velocity(&self, at: MediaTime) -> Option<[f64; 3]> {
        if at < self.start || at > self.end {
            return None;
        }
        let source = self
            .offset
            .checked_add(at.saturating_sub(self.start))
            .ok()?;
        if let Some(global) = &self.global {
            return Some(global.velocity(&self.segments, source).to_array());
        }
        let index = self
            .segments
            .partition_point(|segment| segment.to.at <= source);
        let segment = self.segments.get(index).or_else(|| self.segments.last())?;
        if source < segment.from.at || source > segment.to.at {
            return Some([0.0; 3]);
        }
        let amount =
            (source.0 - segment.from.at.0) as f64 / (segment.to.at.0 - segment.from.at.0) as f64;
        Some(
            (segment.direction(segment.parameter(amount)) * segment.length()
                / segment.to.at.saturating_sub(segment.from.at).seconds())
            .to_array(),
        )
    }

    /// Numerical length diagnostics belong to the compiled rail and do no additional integration.
    pub const fn accuracy(&self) -> PathAccuracy {
        self.accuracy
    }

    /// Bounded overlay work follows the actual traveled segment and never crosses the retained trim.
    pub fn geometry_window(&self, at: MediaTime, maximum: usize) -> std::ops::Range<usize> {
        let Some(end) = self
            .offset
            .checked_add(self.end.saturating_sub(self.start))
            .ok()
        else {
            return 0..0;
        };
        if maximum == 0 {
            return 0..0;
        }
        let source = self
            .offset
            .0
            .saturating_add(at.clamp(self.start, self.end).0 - self.start.0);
        let source = MediaTime(source);
        let (visible, center) = if let Some(global) = &self.global {
            (
                global.segment_range([self.offset, end]),
                global.segment_at(source),
            )
        } else {
            let first = self
                .segments
                .partition_point(|segment| segment.to.at <= self.offset);
            let last = self
                .segments
                .partition_point(|segment| segment.from.at < end);
            (
                first..last,
                self.segments
                    .partition_point(|segment| segment.to.at <= source),
            )
        };
        if visible.is_empty() {
            return visible;
        }
        let start = center
            .saturating_sub(maximum / 2)
            .max(visible.start)
            .min(visible.end.saturating_sub(maximum).max(visible.start));
        start..start.saturating_add(maximum).min(visible.end)
    }

    /// A cached overlay trace clips in traveled distance for global rails and source time otherwise.
    pub fn geometry_samples<const N: usize>(&self, index: usize) -> Option<[[f64; 3]; N]> {
        if N < 2 {
            return None;
        }
        let segment = self.segments.get(index)?;
        let source = [
            self.offset,
            self.offset
                .checked_add(self.end.saturating_sub(self.start))
                .ok()?,
        ];
        let amounts = if let Some(global) = &self.global {
            global.local_amounts(index, source)?
        } else {
            let low = segment.from.at.max(source[0]);
            let high = segment.to.at.min(source[1]);
            if high <= low {
                return None;
            }
            let duration = (segment.to.at.0 - segment.from.at.0) as f64;
            [
                (low.0 - segment.from.at.0) as f64 / duration,
                (high.0 - segment.from.at.0) as f64 / duration,
            ]
        };
        let [low, high] = amounts.map(|amount| segment.parameter(amount));
        Some(std::array::from_fn(|point| {
            cubic(
                segment.controls,
                low + (high - low) * point as f64 / (N - 1) as f64,
            )
            .to_array()
        }))
    }
}

impl ArcSegment {
    /// Rotation and embedded lens values retain their own original key clock in every travel mode.
    fn orientation(&self, elapsed: MediaTime) -> CameraPose {
        let amount = (elapsed.0.saturating_sub(self.from.at.0) as f64
            / (self.to.at.0 - self.from.at.0) as f64)
            .clamp(0.0, 1.0);
        self.from
            .pose
            .interpolated(self.to.pose, self.to.easing.sample(amount))
    }

    /// Geometric subdivision uses bounded stack storage; preparing a rail is proportional to key count.
    fn new(keys: &[CameraKey], index: usize, smooth: bool, continuity: PathContinuity) -> Self {
        let [first, last] = super::path::segment_controls(keys, index, smooth, continuity);
        let controls = [
            DVec3::from_array(keys[index].pose.position),
            DVec3::from_array(first),
            DVec3::from_array(last),
            DVec3::from_array(keys[index + 1].pose.position),
        ];
        let mut lengths = [0.0; ARC_STEPS + 1];
        let mut estimated_error = 0.0;
        for step in 1..=ARC_STEPS {
            let low = (step - 1) as f64 / ARC_STEPS as f64;
            let high = step as f64 / ARC_STEPS as f64;
            let (length, error) = integration::measure(controls, low, high);
            lengths[step] = lengths[step - 1] + length;
            estimated_error += error;
        }
        Self {
            from: keys[index].clone(),
            to: keys[index + 1].clone(),
            controls,
            lengths,
            estimated_error,
        }
    }

    /// Degenerate and stationary segments keep exact endpoints and cannot divide by zero.
    fn sample(&self, elapsed: MediaTime) -> CameraPose {
        let amount = (elapsed.0.saturating_sub(self.from.at.0) as f64
            / (self.to.at.0 - self.from.at.0) as f64)
            .clamp(0.0, 1.0);
        let mut pose = self.orientation(elapsed);
        if amount == 0.0 {
            pose.position = self.from.pose.position;
        } else if amount == 1.0 {
            pose.position = self.to.pose.position;
        } else {
            pose.position = cubic(self.controls, self.parameter(amount)).to_array();
        }
        pose
    }

    /// A monotone lookup inverts traveled distance without integrating forward from previous frames.
    fn parameter(&self, amount: f64) -> f64 {
        if amount <= 0.0 {
            return 0.0;
        }
        if amount >= 1.0 {
            return 1.0;
        }
        let distance = amount * self.lengths[ARC_STEPS];
        let upper = self
            .lengths
            .partition_point(|length| *length < distance)
            .clamp(1, ARC_STEPS);
        let before = self.lengths[upper - 1];
        let length = self.lengths[upper] - before;
        let fraction = if length > 0.0 {
            (distance - before) / length
        } else {
            0.0
        };
        let low = (upper - 1) as f64 / ARC_STEPS as f64;
        let high = upper as f64 / ARC_STEPS as f64;
        integration::invert(
            self.controls,
            low,
            high,
            distance - before,
            length,
            fraction,
        )
    }

    /// Zero-length segments remain admissible and are skipped only by the global distance search.
    fn length(&self) -> f64 {
        self.lengths[ARC_STEPS]
    }

    /// A one-sided endpoint limit handles zero control derivatives without inventing a direction.
    fn direction(&self, parameter: f64) -> DVec3 {
        integration::derivative(self.controls, parameter)
            .try_normalize()
            .unwrap_or_else(|| global::endpoint_direction(self.controls, parameter < 0.5))
    }
}

/// Overlay clipping prepares one active segment table and maps both trim boundaries together.
pub(super) fn geometry_parameters(
    keys: &[CameraKey],
    index: usize,
    smooth: bool,
    continuity: PathContinuity,
    amounts: [f64; 2],
) -> [f64; 2] {
    let segment = ArcSegment::new(keys, index, smooth, continuity);
    amounts.map(|amount| segment.parameter(amount))
}

/// Admission rejects curves whose finite endpoints still overflow their derived travel geometry.
pub(super) fn validate(
    keys: &[CameraKey],
    smooth: bool,
    continuity: PathContinuity,
) -> Result<(), StudioError> {
    let mut total = 0.0;
    for index in 0..keys.len().saturating_sub(1) {
        let segment = ArcSegment::new(keys, index, smooth, continuity);
        total += segment.length();
        if !segment.controls.iter().all(|point| point.is_finite())
            || !segment.lengths.iter().all(|length| length.is_finite())
            || !total.is_finite()
        {
            return Err(StudioError::Invalid(
                "camera path cannot form finite travel distances",
            ));
        }
    }
    Ok(())
}

/// Isolated shot sampling builds only its active segment; frame adapters use the compiled cache above.
pub(super) fn sample(
    keys: &[CameraKey],
    elapsed: MediaTime,
    smooth: bool,
    continuity: PathContinuity,
) -> Option<CameraPose> {
    let index = keys.partition_point(|key| key.at <= elapsed);
    if index == 0 {
        return keys.first().map(|key| key.pose);
    }
    if index == keys.len() {
        return keys.last().map(|key| key.pose);
    }
    Some(ArcSegment::new(keys, index - 1, smooth, continuity).sample(elapsed))
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

    /// Global acceleration is saved independently from per-key rotation and FOV easing.
    pub fn with_path_global_easing(&self, easing: Easing) -> Result<Self, StudioError> {
        if !matches!(self.mode, CameraMode::Path { .. }) {
            return Err(StudioError::Invalid(
                "global travel easing requires a camera rail",
            ));
        }
        let mut next = self.clone();
        next.path_global_easing = easing;
        next.validate()?;
        Ok(next)
    }

    /// Tangent alignment is an explicit geometry edit and never silently rewrites legacy controls.
    pub fn with_path_continuity(&self, continuity: PathContinuity) -> Result<Self, StudioError> {
        if !matches!(self.mode, CameraMode::Path { .. }) {
            return Err(StudioError::Invalid(
                "tangent alignment requires a camera rail",
            ));
        }
        let mut next = self.clone();
        next.path_continuity = continuity;
        next.validate()?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests;
