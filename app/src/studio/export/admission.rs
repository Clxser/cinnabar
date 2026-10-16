//! A sorted boundary sweep admits native replay composition without quadratic clip-pair work.

use std::collections::BTreeMap;

use recording_studio::{
    Project,
    model::{Framing, MediaKind, TransitionKind},
};

#[derive(Clone, Copy)]
enum Channel {
    NativeVisual,
    RasterVisual,
    Shot,
}

#[derive(Default)]
struct Counts {
    native: i32,
    raster: i32,
    shots: i32,
}

/// Unsupported composition fails before encoding; the immutable project remains available for editing.
pub(super) fn prepare(project: &Project, has_device: bool) -> Result<(), String> {
    if !has_device && !project.titles.is_empty() {
        return Err("Titles need an initialized graphics device.".into());
    }
    let kinds: BTreeMap<_, _> = project
        .sources
        .iter()
        .map(|source| (source.id, source.kind))
        .collect();
    let mut events = Vec::new();
    let mut native_clips = std::collections::BTreeSet::new();
    for clip in &project.clips {
        if !clip.enabled {
            continue;
        }
        let native = kinds.get(&clip.source) == Some(&MediaKind::Replay);
        if native {
            if !has_device {
                return Err("Native replay export needs an initialized graphics device.".into());
            }
            if clip.framing != Framing::default() {
                return Err(
                    "Native replay crop/opacity composition is not connected to export yet.".into(),
                );
            }
            native_clips.insert(clip.id);
        }
        let span = clip.span().map_err(|error| error.to_string())?;
        interval(
            &mut events,
            span,
            if native {
                Channel::NativeVisual
            } else {
                Channel::RasterVisual
            },
        );
    }
    for transition in &project.transitions {
        if transition.kind == TransitionKind::DipToBlack {
            continue;
        }
        if native_clips.contains(&transition.from) || native_clips.contains(&transition.to) {
            return Err(
                "Native replay transition composition is not connected to export yet.".into(),
            );
        }
    }
    for shot in &project.shots {
        interval(&mut events, shot.span, Channel::Shot);
    }
    if !has_device
        && (project.effects.iter().any(|effect| effect.enabled)
            || project
                .transitions
                .iter()
                .any(|transition| transition.kind == TransitionKind::DipToBlack))
    {
        return Err("Timeline effects need an initialized graphics device.".into());
    }
    events.sort_unstable_by_key(|event| event.0);
    let mut counts = Counts::default();
    let mut index = 0;
    while index < events.len() {
        let at = events[index].0;
        while index < events.len() && events[index].0 == at {
            let count = match events[index].1 {
                Channel::NativeVisual => &mut counts.native,
                Channel::RasterVisual => &mut counts.raster,
                Channel::Shot => &mut counts.shots,
            };
            *count += events[index].2;
            index += 1;
        }
        if counts.native > 0 && counts.native + counts.raster > 1 {
            return Err("Combining a native replay with another visual layer is not connected to export yet.".into());
        }
        if counts.shots > 0 && counts.native == 0 {
            return Err(
                "Camera shots need a 3D replay clip throughout their timeline range.".into(),
            );
        }
    }
    Ok(())
}

/// Equal-time boundaries are applied together so adjacent half-open clips do not falsely overlap.
fn interval(
    events: &mut Vec<(recording_studio::MediaTime, Channel, i32)>,
    span: recording_studio::Span,
    channel: Channel,
) {
    events.extend([(span.start, channel, 1), (span.end, channel, -1)]);
}
