//! Timeline key gestures preserve ordering, easing and the source clock of speed ramps.
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyKind {
    Camera,
    CameraFov,
    Effect,
    Speed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimelineKey {
    pub item: Id,
    pub kind: KeyKind,
    /// Relative owner time, or absolute source time for a speed key.
    pub at: MediaTime,
}

/// Projects a saved key through its own source clock into an absolute output timestamp.
pub fn key_time(project: &Project, key: TimelineKey) -> Result<MediaTime, StudioError> {
    match key.kind {
        KeyKind::Speed => project
            .clips
            .iter()
            .find(|clip| clip.id == key.item)
            .ok_or(StudioError::Missing {
                kind: "speed clip",
                id: key.item.0,
            })?
            .time_map()?
            .timeline_boundary(key.at),
        _ => item_span(project, key.item)?.start.checked_add(key.at),
    }
}

/// Horizontal dragging never crosses neighbours; vertical motion edits effect amount or speed on a log scale.
pub fn move_key(
    project: &Project,
    key: TimelineKey,
    at: MediaTime,
    value_delta: f64,
) -> Result<(Command, TimelineKey), StudioError> {
    if !value_delta.is_finite() {
        return Err(StudioError::Invalid("key drag must be finite"));
    }
    let span = item_span(project, key.item)?;
    match key.kind {
        KeyKind::Camera => {
            let original = project
                .shots
                .iter()
                .find(|shot| shot.id == key.item)
                .ok_or(StudioError::Missing {
                    kind: "camera",
                    id: key.item.0,
                })?;
            let times = original
                .path_keys()
                .iter()
                .map(|key| key.at)
                .collect::<Vec<_>>();
            let offset = clamp_key(
                &times,
                key.at,
                at.saturating_sub(span.start),
                MediaTime::ZERO,
                span.duration(),
            )?;
            let shot = original.with_key_time(
                span.start.checked_add(key.at)?,
                span.start.checked_add(offset)?,
            )?;
            Ok((
                Command::ReplaceShot { shot },
                TimelineKey { at: offset, ..key },
            ))
        }
        KeyKind::CameraFov => {
            let range = crate::camera::CAMERA_FOV_RANGE;
            let original = project
                .shots
                .iter()
                .find(|shot| shot.id == key.item)
                .ok_or(StudioError::Missing {
                    kind: "camera",
                    id: key.item.0,
                })?;
            let times = original
                .fov_keys()
                .map(|(at, _)| at.saturating_sub(span.start))
                .collect::<Vec<_>>();
            let offset = clamp_key(
                &times,
                key.at,
                at.saturating_sub(span.start),
                MediaTime::ZERO,
                span.duration(),
            )?;
            let old = span.start.checked_add(key.at)?;
            let destination = span.start.checked_add(offset)?;
            let value = original
                .fov_key_at(old)
                .ok_or(StudioError::Invalid("FOV key no longer exists"))?
                .value;
            let value = if value_delta == 0.0 {
                value
            } else {
                (value + value_delta * (range.end - range.start))
                    .clamp(range.start + 0.01, range.end - 0.01)
            };
            let shot = original
                .with_fov_key_time(old, destination)?
                .with_fov_key(destination, value)?;
            Ok((
                Command::ReplaceShot { shot },
                TimelineKey { at: offset, ..key },
            ))
        }
        KeyKind::Effect => {
            let mut effect = project
                .effects
                .iter()
                .find(|effect| effect.id == key.item)
                .cloned()
                .ok_or(StudioError::Missing {
                    kind: "effect",
                    id: key.item.0,
                })?;
            let times = effect
                .amount
                .keys
                .iter()
                .map(|key| key.at)
                .collect::<Vec<_>>();
            let offset = clamp_key(
                &times,
                key.at,
                at.saturating_sub(span.start),
                MediaTime::ZERO,
                span.duration(),
            )?;
            let sample = effect
                .amount
                .keys
                .iter_mut()
                .find(|sample| sample.at == key.at)
                .ok_or(StudioError::Invalid("effect key no longer exists"))?;
            let (minimum, maximum) = effect.kind.limits();
            sample.at = offset;
            sample.value = effect
                .kind
                .constrain(sample.value + value_delta * (maximum - minimum));
            Ok((
                Command::ReplaceEffect { effect },
                TimelineKey { at: offset, ..key },
            ))
        }
        KeyKind::Speed => {
            let clip = project
                .clips
                .iter()
                .find(|clip| clip.id == key.item)
                .ok_or(StudioError::Missing {
                    kind: "clip",
                    id: key.item.0,
                })?;
            let mut ramp = clip
                .speed_ramp
                .clone()
                .ok_or(StudioError::Invalid("clip has no speed keys"))?;
            let times = ramp.keys.iter().map(|key| key.at).collect::<Vec<_>>();
            let source = if key_time(project, key)? == at {
                key.at
            } else {
                clip.time_map()?.source_boundary(at)?
            };
            let offset = clamp_key(
                &times,
                key.at,
                source,
                clip.source_range.start,
                clip.source_range.end,
            )?;
            let sample = ramp
                .keys
                .iter_mut()
                .find(|sample| sample.at == key.at)
                .ok_or(StudioError::Invalid("speed key no longer exists"))?;
            sample.at = offset;
            if value_delta != 0.0 {
                let base = f64::from(clip.rate.numerator) / f64::from(clip.rate.denominator);
                let minimum = (1.0 / 16.0 / base).max(1.0 / 16.0);
                let maximum = (16.0 / base).min(16.0);
                let multiplier = (f64::from(sample.rate.numerator)
                    / f64::from(sample.rate.denominator)
                    * 2.0_f64.powf(value_delta * 8.0))
                .clamp(minimum, maximum);
                sample.rate = crate::PlaybackRate {
                    numerator: (multiplier * 65_536.0)
                        .round()
                        .clamp((minimum * 65_536.0).ceil(), (maximum * 65_536.0).floor())
                        as u32,
                    denominator: 65_536,
                };
            }
            ramp.validate(clip.rate)?;
            Ok((
                Command::SetSpeedRamp {
                    id: clip.id,
                    ramp: Some(ramp),
                    ripple: false,
                },
                TimelineKey { at: offset, ..key },
            ))
        }
    }
}

/// At least one microsecond remains between adjacent keys, with no replacement of a neighbour.
fn clamp_key(
    times: &[MediaTime],
    old: MediaTime,
    wanted: MediaTime,
    minimum: MediaTime,
    maximum: MediaTime,
) -> Result<MediaTime, StudioError> {
    let index = times
        .binary_search(&old)
        .map_err(|_| StudioError::Invalid("key no longer exists"))?;
    let low = index
        .checked_sub(1)
        .map_or(minimum.0, |index| times[index].0 + 1)
        .max(minimum.0);
    let high = times
        .get(index + 1)
        .map_or(maximum.0, |at| at.0.saturating_sub(1))
        .min(maximum.0);
    if low > high {
        return Err(StudioError::Invalid(
            "no space remains between neighbouring keys",
        ));
    }
    Ok(MediaTime(wanted.0.clamp(low, high)))
}
