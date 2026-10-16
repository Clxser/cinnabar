//! A sorted boundary sweep admits native replay composition without quadratic clip-pair work.

use std::collections::BTreeMap;

use recording_studio::{
    Project,
    effect::EffectKind,
    model::{Framing, MediaKind, TransitionKind},
};

#[derive(Clone, Copy)]
enum Channel {
    NativeVisual,
    RasterVisual,
    Shot,
    CameraEffect,
}

#[derive(Default)]
struct Counts {
    native: i32,
    raster: i32,
    shots: i32,
    camera_effects: i32,
}

/// Unsupported composition fails before encoding; the immutable project remains available for editing.
pub(super) fn prepare(project: &Project, has_device: bool) -> Result<(), String> {
    if !project.titles.is_empty() {
        return Err(
            "Title rendering is not connected to export yet; remove titles to export this project."
                .into(),
        );
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
            return Err("Dip-to-black composition is not connected to export yet.".into());
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
    for effect in &project.effects {
        if !effect.enabled {
            continue;
        }
        match effect.kind {
            EffectKind::Zoom | EffectKind::Shake => {
                interval(&mut events, effect.span, Channel::CameraEffect)
            }
            EffectKind::Exposure
            | EffectKind::Saturation
            | EffectKind::Vignette
            | EffectKind::Bloom
            | EffectKind::Flash
            | EffectKind::Letterbox
            | EffectKind::Fade => {
                return Err("Color/effect composition is not connected to export yet; unsupported edits were not silently omitted.".into());
            }
        }
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
                Channel::CameraEffect => &mut counts.camera_effects,
            };
            *count += events[index].2;
            index += 1;
        }
        if counts.native > 0 && counts.native + counts.raster > 1 {
            return Err("Combining a native replay with another visual layer is not connected to export yet.".into());
        }
        if counts.shots + counts.camera_effects > 0 && counts.native == 0 {
            return Err("Camera shots and camera effects need a 3D replay clip throughout their timeline range.".into());
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
