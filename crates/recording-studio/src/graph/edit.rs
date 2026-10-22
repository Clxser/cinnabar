//! Graph commands modify one saved owner; previews and undo use the same admitted document.

use super::*;
use crate::retime::{SpeedKey, SpeedRamp};

const SPEED_GRAPH_DENOMINATOR: u32 = 65_536;

/// Fixed graph precision rounds only into the legal multiplier and combined-rate rational interval.
fn graph_rate(value: f64, base: crate::PlaybackRate) -> crate::PlaybackRate {
    let denominator = u128::from(SPEED_GRAPH_DENOMINATOR);
    let maximum = u128::from(crate::time::MAX_PLAYBACK_RATE);
    let low = denominator.div_ceil(maximum).max(
        (denominator * u128::from(base.denominator)).div_ceil(u128::from(base.numerator) * maximum),
    );
    let high = (denominator * maximum)
        .min(denominator * u128::from(base.denominator) * maximum / u128::from(base.numerator));
    let numerator =
        ((value.exp2() * f64::from(SPEED_GRAPH_DENOMINATOR)).round() as u128).clamp(low, high);
    crate::PlaybackRate {
        numerator: numerator as u32,
        denominator: SPEED_GRAPH_DENOMINATOR,
    }
}

/// An existing displayed key resolves to its retained source timestamp, avoiding round-trip retime drift.
fn source_key(graph: &GraphCurve, at: MediaTime) -> Result<MediaTime, StudioError> {
    let mut found = graph.points.iter().filter(|p| p.at == at);
    let source = found
        .next()
        .ok_or(StudioError::Invalid("select an existing curve key"))?
        .source_at;
    if found.next().is_some() {
        return Err(StudioError::Invalid(
            "zoom in and separate coincident source keys first",
        ));
    }
    Ok(source)
}

/// Timeline coordinates become owner-relative or source time only at the editing boundary.
fn source_at(graph: &GraphCurve, at: MediaTime) -> Result<MediaTime, StudioError> {
    if at < graph.span.start || at > graph.span.end {
        return Err(StudioError::Invalid("curve key is outside its owner"));
    }
    if let Some(map) = &graph.map {
        map.source_boundary(at)
    } else {
        graph
            .offset
            .checked_add(at.saturating_sub(graph.span.start))
    }
}

/// Produces one ordinary replace command with the same admission used by project load and export.
fn command(project: &Project, graph: &GraphCurve, curve: Curve) -> Result<Command, StudioError> {
    match graph.channel {
        GraphChannel::Camera(channel) => {
            let mut shot = project
                .shots
                .iter()
                .find(|s| s.id == graph.item)
                .unwrap()
                .clone();
            shot.animation.curves.insert(
                channel,
                crate::camera::CameraChannelCurve {
                    curve,
                    offset: graph.offset,
                    additive: graph.additive,
                },
            );
            shot.validate()?;
            Ok(Command::ReplaceShot { shot })
        }
        GraphChannel::Fov => {
            let mut shot = project
                .shots
                .iter()
                .find(|s| s.id == graph.item)
                .unwrap()
                .clone();
            shot.fov_curve = Some(curve);
            shot.validate()?;
            Ok(Command::ReplaceShot { shot })
        }
        GraphChannel::Effect => {
            let mut effect = project
                .effects
                .iter()
                .find(|s| s.id == graph.item)
                .unwrap()
                .clone();
            effect.amount = curve;
            effect.amount.validate(effect.span.duration())?;
            Ok(Command::ReplaceEffect { effect })
        }
        GraphChannel::Speed => {
            let clip = project.clips.iter().find(|s| s.id == graph.item).unwrap();
            let ramp = SpeedRamp {
                keys: curve
                    .keys
                    .into_iter()
                    .map(|key| SpeedKey {
                        at: key.at,
                        easing: key.easing,
                        rate: clip
                            .speed_ramp
                            .as_ref()
                            .and_then(|ramp| {
                                ramp.keys.iter().find(|old| {
                                    (f64::from(old.rate.numerator)
                                        / f64::from(old.rate.denominator))
                                    .log2()
                                        == key.value
                                })
                            })
                            .map_or_else(|| graph_rate(key.value, clip.rate), |old| old.rate),
                    })
                    .collect(),
            };
            ramp.validate(clip.rate)?;
            Ok(Command::SetSpeedRamp {
                id: graph.item,
                ramp: Some(ramp),
                ripple: false,
            })
        }
    }
}

/// Drags change key time/value while retaining neighbours and all unedited easing and source support.
pub fn set_key(
    project: &Project,
    item: Id,
    channel: GraphChannel,
    old: Option<MediaTime>,
    at: MediaTime,
    value: f64,
    base: CameraPose,
) -> Result<Command, StudioError> {
    let graph = GraphCurve::new(project, item, channel, base)?;
    set_key_on(&graph, graph.project.as_ref(), old, at, value)
}

/// A pointer drag edits its frozen admitted graph without rebuilding positional arc tables.
pub fn set_key_on(
    graph: &GraphCurve,
    project: &Project,
    old: Option<MediaTime>,
    at: MediaTime,
    value: f64,
) -> Result<Command, StudioError> {
    graph.require_project(project)?;
    let (item, channel) = (graph.item, graph.channel);
    if !value.is_finite() {
        return Err(StudioError::Invalid("curve value must be finite"));
    }
    let mut destination = source_at(graph, at)?;
    let mut curve = graph.curve.clone();
    let value = match channel {
        GraphChannel::Camera(CameraChannel::Pitch) if !graph.relative_values() => {
            value.clamp(-90.0, 90.0)
        }
        GraphChannel::Fov => value.clamp(
            crate::camera::CAMERA_FOV_RANGE.start + 0.001,
            crate::camera::CAMERA_FOV_RANGE.end - 0.001,
        ),
        GraphChannel::Effect => project
            .effects
            .iter()
            .find(|s| s.id == item)
            .unwrap()
            .kind
            .constrain(value),
        GraphChannel::Speed => {
            let clip = project.clips.iter().find(|s| s.id == item).unwrap();
            let rate = f64::from(clip.rate.numerator) / f64::from(clip.rate.denominator);
            let maximum = f64::from(crate::time::MAX_PLAYBACK_RATE);
            value.clamp(
                (1.0 / maximum / rate).max(1.0 / maximum).log2(),
                (maximum / rate).min(maximum).log2(),
            )
        }
        _ => value,
    };
    let easing = if let Some(old) = old {
        let source = source_key(graph, old)?;
        let index = curve.keys.binary_search_by_key(&source, |k| k.at).unwrap();
        let low = index.checked_sub(1).map_or(0, |i| curve.keys[i].at.0 + 1);
        let high = curve
            .keys
            .get(index + 1)
            .map_or(crate::time::MAX_MEDIA_MICROS, |k| k.at.0.saturating_sub(1));
        if low > high {
            return Err(StudioError::Invalid(
                "there is no space between adjacent curve keys",
            ));
        }
        destination.0 = destination.0.clamp(low, high);
        curve.keys.remove(index).easing
    } else {
        Easing::Smooth
    };
    let time = if let Some(map) = &graph.map {
        map.timeline_boundary(destination)?
    } else {
        let time =
            i128::from(graph.span.start.0) + i128::from(destination.0) - i128::from(graph.offset.0);
        MediaTime(u64::try_from(time).map_err(|_| StudioError::TimeOverflow)?)
    };
    let value = graph.authored_value(time, value)?;
    match curve.keys.binary_search_by_key(&destination, |k| k.at) {
        Ok(index) => curve.keys[index].value = value,
        Err(index) => curve.keys.insert(
            index,
            CurveKey {
                at: destination,
                value,
                easing,
            },
        ),
    }
    command(project, graph, curve)
}

/// Deleting the last scalar camera key restores the underlying rig; effect and speed owners keep one key.
pub fn delete_key(
    project: &Project,
    key: GraphKey,
    base: CameraPose,
) -> Result<Command, StudioError> {
    let graph = GraphCurve::new(project, key.item, key.channel, base)?;
    let source = source_key(&graph, key.at)?;
    let mut curve = graph.curve.clone();
    if curve.keys.len() == 1 {
        return reset(project, key.item, key.channel);
    }
    curve.keys.retain(|k| k.at != source);
    command(project, &graph, curve)
}

/// Easing and tangent edits belong to the segment arriving at the exact selected key.
pub fn set_easing(
    project: &Project,
    key: GraphKey,
    easing: Easing,
    base: CameraPose,
) -> Result<Command, StudioError> {
    let graph = GraphCurve::new(project, key.item, key.channel, base)?;
    set_easing_on(&graph, graph.project.as_ref(), key, easing)
}

/// Tangent drags share a frozen source graph and reject stale document or selection identities.
pub fn set_easing_on(
    graph: &GraphCurve,
    project: &Project,
    key: GraphKey,
    easing: Easing,
) -> Result<Command, StudioError> {
    graph.require_project(project)?;
    if graph.item != key.item || graph.channel != key.channel {
        return Err(StudioError::Invalid(
            "curve key belongs to a different graph",
        ));
    }
    let source = source_key(graph, key.at)?;
    let mut curve = graph.curve.clone();
    curve
        .keys
        .iter_mut()
        .find(|k| k.at == source)
        .unwrap()
        .easing = easing;
    command(project, graph, curve)
}

/// Reset removes a camera override or restores one neutral owner curve through the regular undo command.
pub fn reset(project: &Project, item: Id, channel: GraphChannel) -> Result<Command, StudioError> {
    match channel {
        GraphChannel::Camera(channel) => {
            let mut shot = project
                .shots
                .iter()
                .find(|s| s.id == item)
                .cloned()
                .ok_or(StudioError::Invalid("select a camera shot"))?;
            shot.animation.curves.remove(&channel);
            Ok(Command::ReplaceShot { shot })
        }
        GraphChannel::Fov => {
            let mut shot = project
                .shots
                .iter()
                .find(|s| s.id == item)
                .cloned()
                .ok_or(StudioError::Invalid("select a camera shot"))?;
            shot.fov_curve = None;
            shot.fov_offset = MediaTime::ZERO;
            Ok(Command::ReplaceShot { shot })
        }
        GraphChannel::Effect => {
            let mut effect = project
                .effects
                .iter()
                .find(|s| s.id == item)
                .cloned()
                .ok_or(StudioError::Invalid("select an effect"))?;
            effect.amount = Curve::constant(effect.kind.initial());
            Ok(Command::ReplaceEffect { effect })
        }
        GraphChannel::Speed => Ok(Command::SetSpeedRamp {
            id: item,
            ramp: None,
            ripple: false,
        }),
    }
}
