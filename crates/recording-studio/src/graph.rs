//! Scalar graph editing shares saved curves with playback, including weighted easing and native source clocks.

use crate::{
    Id, MediaTime, Project, Span, StudioError,
    camera::{CameraChannel, CameraPose},
    edit::Command,
    effect::{Curve, CurveKey, Easing},
};

mod edit;
#[cfg(test)]
mod tests;
pub use edit::{delete_key, reset, set_easing, set_key};

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

/// Existing path keys seed a new independent channel; unchanged shots keep their original rig behavior.
pub(super) fn camera_curve(
    shot: &crate::camera::CameraShot,
    channel: CameraChannel,
    base: CameraPose,
) -> (Curve, MediaTime) {
    if let Some(curve) = shot.animation.curves.get(&channel) {
        return (curve.curve.clone(), curve.offset);
    }
    let keys = shot
        .path_keys()
        .iter()
        .map(|key| CurveKey {
            at: key.at,
            value: channel.value(key.pose),
            easing: key.easing,
        })
        .collect::<Vec<_>>();
    if keys.is_empty() {
        (Curve::constant(channel.value(base)), MediaTime::ZERO)
    } else {
        (Curve { keys }, shot.path_offset)
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
        if !channels(project, item).contains(&channel) {
            return Err(StudioError::Invalid(
                "select an owner with an editable curve",
            ));
        }
        let (span, curve, offset, map) = match channel {
            GraphChannel::Camera(channel) => {
                let shot = project.shots.iter().find(|s| s.id == item).unwrap();
                let (curve, offset) = camera_curve(shot, channel, base);
                (shot.span, curve, offset, None)
            }
            GraphChannel::Fov => {
                let shot = project.shots.iter().find(|s| s.id == item).unwrap();
                (
                    shot.span,
                    shot.fov_curve
                        .clone()
                        .unwrap_or_else(|| Curve::constant(base.fov)),
                    shot.fov_offset,
                    None,
                )
            }
            GraphChannel::Effect => {
                let effect = project.effects.iter().find(|s| s.id == item).unwrap();
                (effect.span, effect.amount.clone(), MediaTime::ZERO, None)
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
                (map.span(), curve, MediaTime::ZERO, Some(map))
            }
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
        let (mut low, mut high) = points
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| {
                (a.min(p.value), b.max(p.value))
            });
        if !low.is_finite() {
            low = curve.sample(offset);
            high = low;
        }
        let pad = (high * 0.15 - low * 0.15).abs().max(match channel {
            GraphChannel::Fov => 5.0,
            GraphChannel::Speed => 0.5,
            _ => 1.0,
        });
        low = (low - pad).max(-f64::MAX);
        high = (high + pad).min(f64::MAX);
        let rate_scale = project
            .clips
            .iter()
            .find(|clip| clip.id == item)
            .map_or(1.0, |clip| {
                f64::from(clip.rate.numerator) / f64::from(clip.rate.denominator)
            });
        Ok(Self {
            item,
            channel,
            span,
            points,
            range: [low, high],
            curve,
            offset,
            map,
            rate_scale,
        })
    }

    /// Speed graphs display their evaluated multiplier in stops, including weighted source easing.
    pub fn sample(&self, at: MediaTime) -> f64 {
        if let Some(map) = &self.map {
            return (map.rate_at(at) / self.rate_scale).log2();
        }
        self.curve.sample(MediaTime(
            self.offset
                .0
                .saturating_add(at.saturating_sub(self.span.start).0),
        ))
    }

    /// Projects weighted controls for the segment arriving at the selected key.
    pub fn handles(&self, at: MediaTime) -> Option<[[f64; 2]; 2]> {
        let index = self.points.binary_search_by_key(&at, |p| p.at).ok()?;
        let previous = self.points.get(index.checked_sub(1)?)?;
        let point = self.points[index];
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
                [
                    previous.at.0 as f64 + (point.at.0 - previous.at.0) as f64 * p[0],
                    previous.value * (1.0 - p[1]) + point.value * p[1],
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
            [
                (position[0] - a.at.0 as f64) / (b.at.0 - a.at.0) as f64,
                if (b.value - a.value).abs() > 1e-12 {
                    (position[1] - a.value) / (b.value - a.value)
                } else {
                    0.5
                },
            ]
        };
        [x, y].into_iter().all(f64::is_finite).then_some([x, y])
    }
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
