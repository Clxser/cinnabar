//! Scalar graph editing shares saved curves with playback, including weighted easing and native source clocks.

use crate::{
    Id, MediaTime, Project, Span, StudioError,
    camera::{CameraChannel, CameraPose},
    edit::Command,
    effect::{Curve, CurveKey, Easing, scalar_lerp},
};
use std::sync::Arc;

mod base;
mod edit;
#[cfg(test)]
mod tests;
pub use edit::{delete_key, reset, set_easing, set_easing_on, set_key, set_key_on};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphChannel {
    Camera(CameraChannel),
    Fov,
    Effect,
    Speed,
}

impl GraphChannel {
    /// A graph channel names its saved value, independent of visual lane or key ordering.
    pub fn label(self) -> &'static str {
        match self {
            Self::Camera(channel) => channel.label(),
            Self::Fov => "FOV",
            Self::Effect => "Amount",
            Self::Speed => "Speed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphKey {
    pub item: Id,
    pub channel: GraphChannel,
    pub at: MediaTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphControl {
    Key,
    Outgoing,
    Incoming,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphPoint {
    pub at: MediaTime,
    pub source_at: MediaTime,
    pub value: f64,
    pub easing: Easing,
}

/// One immutable graph publication is prepared per selection/document change, never per painted point.
pub struct GraphCurve {
    pub item: Id,
    pub channel: GraphChannel,
    pub span: Span,
    pub points: Vec<GraphPoint>,
    pub range: [f64; 2],
    curve: Curve,
    offset: MediaTime,
    map: Option<crate::retime::TimeMap>,
    rate_scale: f64,
    project: Arc<Project>,
    base: Option<Arc<base::BaseRig>>,
    additive: bool,
}

/// Applicable channels derive from the selected saved owner rather than duplicated UI lookup tables.
pub fn channels(project: &Project, item: Id) -> Vec<GraphChannel> {
    if project.shots.iter().any(|shot| shot.id == item) {
        CameraChannel::ALL
            .iter()
            .copied()
            .map(GraphChannel::Camera)
            .chain([GraphChannel::Fov])
            .collect()
    } else if project.effects.iter().any(|effect| effect.id == item) {
        vec![GraphChannel::Effect]
    } else if project
        .clips
        .iter()
        .any(|clip| clip.id == item && clip.speed_ramp.is_some())
    {
        vec![GraphChannel::Speed]
    } else {
        Vec::new()
    }
}

/// Zero offsets retain every original path support key and its phase without approximating its geometry.
pub(super) fn camera_curve(
    shot: &crate::camera::CameraShot,
    channel: CameraChannel,
) -> (Curve, MediaTime, bool) {
    if let Some(curve) = shot.animation.curves.get(&channel) {
        return (curve.curve.clone(), curve.offset, curve.additive);
    }
    let keys = shot
        .path_keys()
        .iter()
        .map(|key| CurveKey {
            at: key.at,
            value: 0.0,
            easing: key.easing,
        })
        .collect::<Vec<_>>();
    if keys.is_empty() {
        (Curve::constant(0.0), MediaTime::ZERO, true)
    } else {
        (Curve { keys }, shot.path_offset, true)
    }
}

impl GraphCurve {
    /// Admits only the channel belonging to this owner and retains the exact source-time map.
    pub fn new(
        project: &Project,
        item: Id,
        channel: GraphChannel,
        base: CameraPose,
    ) -> Result<Self, StudioError> {
        Self::new_reusing(Arc::new(project.clone()), item, channel, base, None)
    }

    /// Retains the exact document and reuses rail geometry while refreshing all authored graph metadata.
    pub fn new_reusing(
        project: Arc<Project>,
        item: Id,
        channel: GraphChannel,
        _base: CameraPose,
        previous: Option<&Self>,
    ) -> Result<Self, StudioError> {
        if !channels(&project, item).contains(&channel) {
            return Err(StudioError::Invalid(
                "select an owner with an editable curve",
            ));
        }
        let (span, curve, offset, map, additive) = match channel {
            GraphChannel::Camera(channel) => {
                let shot = project.shots.iter().find(|s| s.id == item).unwrap();
                shot.validate()?;
                let (curve, offset, additive) = camera_curve(shot, channel);
                (shot.span, curve, offset, None, additive)
            }
            GraphChannel::Fov => {
                let shot = project.shots.iter().find(|s| s.id == item).unwrap();
                shot.validate()?;
                let (curve, offset) = shot.authored_fov_curve();
                (shot.span, curve, offset, None, false)
            }
            GraphChannel::Effect => {
                let effect = project.effects.iter().find(|s| s.id == item).unwrap();
                (
                    effect.span,
                    effect.amount.clone(),
                    MediaTime::ZERO,
                    None,
                    false,
                )
            }
            GraphChannel::Speed => {
                let clip = project.clips.iter().find(|s| s.id == item).unwrap();
                let map = clip.time_map()?;
                let curve = Curve {
                    keys: clip
                        .speed_ramp
                        .as_ref()
                        .unwrap()
                        .keys
                        .iter()
                        .map(|key| CurveKey {
                            at: key.at,
                            value: (f64::from(key.rate.numerator)
                                / f64::from(key.rate.denominator))
                            .log2(),
                            easing: key.easing,
                        })
                        .collect(),
                };
                (map.span(), curve, MediaTime::ZERO, Some(map), false)
            }
        };
        let camera_base = if matches!(channel, GraphChannel::Camera(_)) && additive {
            let shot = project.shots.iter().find(|shot| shot.id == item).unwrap();
            Some(base::BaseRig::new(
                shot,
                previous.and_then(|old| old.base.as_ref()),
            ))
        } else {
            None
        };
        let points: Vec<_> = curve
            .keys
            .iter()
            .filter_map(|key| {
                let at = if let Some(map) = &map {
                    map.timeline_boundary(key.at).ok()?
                } else {
                    let at = i128::from(span.start.0) + i128::from(key.at.0) - i128::from(offset.0);
                    if at < 0 {
                        return None;
                    }
                    MediaTime(at as u64)
                };
                (at >= span.start && at <= span.end).then_some(GraphPoint {
                    at,
                    source_at: key.at,
                    value: key.value,
                    easing: key.easing,
                })
            })
            .collect();
        let rate_scale = project
            .clips
            .iter()
            .find(|clip| clip.id == item)
            .map_or(1.0, |clip| {
                f64::from(clip.rate.numerator) / f64::from(clip.rate.denominator)
            });
        let mut graph = Self {
            item,
            channel,
            span,
            points,
            range: [0.0, 1.0],
            curve,
            offset,
            map,
            rate_scale,
            project,
            base: camera_base,
            additive,
        };
        let values = graph
            .points
            .iter()
            .map(|point| graph.display_value(point.at, point.value))
            .collect::<Vec<_>>();
        for (point, value) in graph.points.iter_mut().zip(values) {
            point.value = value;
        }
        graph.refresh_range();
        Ok(graph)
    }

    /// Target-bound additive curves are authored as offsets; no historical subject pose is guessed.
    pub fn relative_values(&self) -> bool {
        self.additive
            && self
                .base
                .as_ref()
                .is_some_and(|base| base.subject_relative())
    }

    /// Stable normalization avoids overflowing the difference between finite opposite endpoints.
    pub fn range_fraction(&self, value: f64) -> f64 {
        range_fraction(self.range, value)
    }

    /// Convex interpolation keeps even an extreme finite graph range representable.
    pub fn range_value(&self, fraction: f64) -> f64 {
        range_value(self.range, fraction)
    }

    /// Frozen drags belong to one retained document; stale graphs never overwrite a newer owner.
    pub(super) fn require_project(&self, project: &Project) -> Result<(), StudioError> {
        if std::ptr::eq(project, self.project.as_ref()) {
            Ok(())
        } else {
            Err(StudioError::Invalid(
                "curve graph belongs to a different document snapshot",
            ))
        }
    }

    /// Converts authored offsets to displayed values through the original, unmodified rig evaluator.
    fn display_value(&self, at: MediaTime, value: f64) -> f64 {
        if self.additive && !self.relative_values() {
            if let GraphChannel::Camera(channel) = self.channel {
                return channel.offset_value(self.base.as_ref().unwrap().value(channel, at), value);
            }
        }
        value
    }

    /// Absolute Free/Path edits become finite deltas; subject rigs already expose delta units.
    pub(super) fn authored_value(&self, at: MediaTime, value: f64) -> Result<f64, StudioError> {
        let value = if self.additive && !self.relative_values() {
            let GraphChannel::Camera(channel) = self.channel else {
                unreachable!()
            };
            value - self.base.as_ref().unwrap().value(channel, at)
        } else {
            value
        };
        value
            .is_finite()
            .then_some(value)
            .ok_or(StudioError::Invalid(
                "camera offset cannot form a finite difference",
            ))
    }

    /// Exact source keys resolve in logarithmic work even on a maximum-size rail.
    fn raw_key(&self, at: MediaTime) -> Option<CurveKey> {
        self.curve
            .keys
            .binary_search_by_key(&at, |key| key.at)
            .ok()
            .map(|index| self.curve.keys[index])
    }

    /// Visible endpoints and support extrema bound the whole evaluated curve, even without visible keys.
    fn refresh_range(&mut self) {
        let source_end = MediaTime(self.offset.0 + self.span.duration().0);
        let (a, b) = (
            self.curve.sample(self.offset),
            self.curve.sample(source_end),
        );
        let mut bounds = [a.min(b), a.max(b)];
        for key in &self.curve.keys {
            if key.at >= self.offset && key.at <= source_end {
                bounds = [bounds[0].min(key.value), bounds[1].max(key.value)];
            }
        }
        if self.map.is_some() {
            let (a, b) = (self.sample(self.span.start), self.sample(self.span.end));
            bounds = [a.min(b), a.max(b)];
            for point in &self.points {
                bounds = [bounds[0].min(point.value), bounds[1].max(point.value)];
            }
        } else if self.additive && !self.relative_values() {
            if let GraphChannel::Camera(channel) = self.channel {
                let base = self.base.as_ref().unwrap().bounds(channel);
                bounds = [
                    channel.offset_value(base[0], bounds[0]),
                    channel.offset_value(base[1], bounds[1]),
                ];
            }
        }
        let pad = (bounds[1] * 0.15 - bounds[0] * 0.15)
            .abs()
            .max(match self.channel {
                GraphChannel::Fov => 5.0,
                GraphChannel::Speed => 0.5,
                _ => 1.0,
            });
        self.range = [
            (bounds[0] - pad).max(-f64::MAX),
            (bounds[1] + pad).min(f64::MAX),
        ];
        if self.range[0] == self.range[1] {
            let value = self.range[0];
            self.range = if value > 0.0 {
                [value * 0.5, value]
            } else if value < 0.0 {
                [value, value * 0.5]
            } else {
                [-1.0, 1.0]
            };
        }
    }

    /// Speed graphs display their evaluated multiplier in stops, including weighted source easing.
    pub fn sample(&self, at: MediaTime) -> f64 {
        if let Some(map) = &self.map {
            return (map.rate_at(at) / self.rate_scale).log2();
        }
        let at = at.max(self.span.start).min(self.span.end);
        self.display_value(
            at,
            self.curve.sample(MediaTime(
                self.offset
                    .0
                    .saturating_add(at.saturating_sub(self.span.start).0),
            )),
        )
    }

    /// Controls shape the arriving scalar offset over the original rig, not an approximate baked rail.
    pub fn handles(&self, at: MediaTime) -> Option<[[f64; 2]; 2]> {
        let index = self.points.binary_search_by_key(&at, |p| p.at).ok()?;
        let previous = self.points.get(index.checked_sub(1)?)?;
        let point = self.points[index];
        let a = self.raw_key(previous.source_at)?.value;
        let b = self.raw_key(point.source_at)?.value;
        let (outgoing, incoming) = controls(point.easing);
        Some([outgoing, incoming].map(|p| {
            if let Some(map) = &self.map {
                let source = previous.source_at.0 as f64
                    + (point.source_at.0 - previous.source_at.0) as f64 * p[0];
                let time = map
                    .timeline_boundary(MediaTime(source.round() as u64))
                    .unwrap_or(previous.at);
                let inverse =
                    (-previous.value).exp2() * (1.0 - p[1]) + (-point.value).exp2() * p[1];
                [time.0 as f64, -inverse.log2()]
            } else {
                let time = previous.at.0 as f64 + (point.at.0 - previous.at.0) as f64 * p[0];
                [
                    time,
                    self.display_value(MediaTime(time.round() as u64), scalar_lerp(a, b, p[1])),
                ]
            }
        }))
    }

    /// Speed controls transform through source time and inverse rate, matching their analytic time map.
    pub fn handle_input(&self, at: MediaTime, position: [f64; 2]) -> Option<[f64; 2]> {
        if !position.into_iter().all(f64::is_finite) {
            return None;
        }
        let index = self.points.binary_search_by_key(&at, |p| p.at).ok()?;
        let a = self.points.get(index.checked_sub(1)?)?;
        let b = self.points[index];
        let [x, y] = if let Some(map) = &self.map {
            let time = MediaTime(
                position[0]
                    .clamp(self.span.start.0 as f64, self.span.end.0 as f64)
                    .round() as u64,
            );
            let source = map.source_boundary(time).ok()?;
            let first = (-a.value).exp2();
            let last = (-b.value).exp2();
            [
                (source.0 as f64 - a.source_at.0 as f64) / (b.source_at.0 - a.source_at.0) as f64,
                if (last - first).abs() > 1e-12 {
                    ((-position[1]).exp2() - first) / (last - first)
                } else {
                    0.5
                },
            ]
        } else {
            let first = self.raw_key(a.source_at)?.value;
            let last = self.raw_key(b.source_at)?.value;
            let at = MediaTime(
                position[0]
                    .clamp(self.span.start.0 as f64, self.span.end.0 as f64)
                    .round() as u64,
            );
            let value = self.authored_value(at, position[1]).ok()?;
            [
                (position[0] - a.at.0 as f64) / (b.at.0 - a.at.0) as f64,
                if first != last {
                    fraction(first, last, value)
                } else {
                    0.5
                },
            ]
        };
        [x, y].into_iter().all(f64::is_finite).then_some([x, y])
    }
}

/// Dividing first keeps opposite extreme finite values from overflowing a graph's normalized span.
fn fraction(first: f64, last: f64, value: f64) -> f64 {
    let scale = first.abs().max(last.abs()).max(value.abs());
    if scale == 0.0 || first == last {
        return 0.5;
    }
    (value / scale - first / scale) / (last / scale - first / scale)
}

/// Finite ranges normalize safely; off-range values remain off-range for actual segment clipping.
pub fn range_fraction(range: [f64; 2], value: f64) -> f64 {
    fraction(range[0], range[1], value).clamp(-f64::MAX, f64::MAX)
}

/// Editor pan and zoom ranges use the same bounded finite interpolation as saved scalar curves.
pub fn range_value(range: [f64; 2], fraction: f64) -> f64 {
    scalar_lerp(range[0], range[1], fraction.clamp(0.0, 1.0))
}

/// Fixed-point handles retain existing named easing exactly when converted to a cubic graph.
pub fn controls(easing: Easing) -> ([f64; 2], [f64; 2]) {
    match easing {
        Easing::Bezier { outgoing, incoming } => (
            outgoing.map(|v| f64::from(v) / f64::from(u16::MAX)),
            incoming.map(|v| f64::from(v) / f64::from(u16::MAX)),
        ),
        Easing::Smooth => ([1.0 / 3.0, 0.0], [2.0 / 3.0, 1.0]),
        Easing::EaseIn => ([1.0 / 3.0, 0.0], [2.0 / 3.0, 0.0]),
        Easing::EaseOut => ([1.0 / 3.0, 1.0], [2.0 / 3.0, 1.0]),
        Easing::Linear | Easing::Hold => ([1.0 / 3.0, 1.0 / 3.0], [2.0 / 3.0, 2.0 / 3.0]),
    }
}

/// Normalized control drags remain inside the segment's finite value envelope.
pub fn with_handle(
    easing: Easing,
    control: GraphControl,
    point: [f64; 2],
) -> Result<Easing, StudioError> {
    if !point.into_iter().all(f64::is_finite) {
        return Err(StudioError::Invalid("curve handles must be finite"));
    }
    let (a, b) = controls(easing);
    let pack = |p: [f64; 2]| p.map(|v| (v.clamp(0.0, 1.0) * f64::from(u16::MAX)).round() as u16);
    Ok(Easing::Bezier {
        outgoing: pack(if control == GraphControl::Outgoing {
            point
        } else {
            a
        }),
        incoming: pack(if control == GraphControl::Incoming {
            point
        } else {
            b
        }),
    })
}
