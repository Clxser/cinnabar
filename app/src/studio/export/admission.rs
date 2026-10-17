//! Linear boundary admission retains device and camera coverage checks for the complete export.

use recording_studio::{
    Id, MediaTime, Project, Span,
    model::{MediaKind, TransitionKind},
};
use std::collections::BTreeMap;

/// Crops, alpha, native/raster overlaps and dissolve groups share the compositor and need no rejection.
/// Camera gaps still fail before encoding, including recorded handles exposed by transitions.
pub(super) fn prepare(project: &Project, has_device: bool) -> Result<(), String> {
    let kinds: BTreeMap<_, _> = project
        .sources
        .iter()
        .map(|source| (source.id, source.kind))
        .collect();
    let mut native: BTreeMap<Id, Span> = BTreeMap::new();
    let clips: BTreeMap<_, _> = project.clips.iter().map(|clip| (clip.id, clip)).collect();
    for clip in project.clips.iter().filter(|clip| clip.enabled) {
        if kinds.get(&clip.source) == Some(&MediaKind::Replay) {
            if !has_device {
                return Err("Native replay export needs an initialized graphics device.".into());
            }
            native.insert(clip.id, clip.span().map_err(|error| error.to_string())?);
        }
    }
    if !has_device
        && (!project.titles.is_empty()
            || project.effects.iter().any(|effect| effect.enabled)
            || project
                .transitions
                .iter()
                .any(|transition| transition.kind == TransitionKind::DipToBlack))
    {
        return Err("Timeline effects and titles need an initialized graphics device.".into());
    }
    for transition in project
        .transitions
        .iter()
        .filter(|transition| transition.kind == TransitionKind::Crossfade)
    {
        let from = clips
            .get(&transition.from)
            .ok_or("A transition source clip is missing.")?;
        let to = clips
            .get(&transition.to)
            .ok_or("A transition destination clip is missing.")?;
        if !from.enabled || !to.enabled {
            continue;
        }
        let start = MediaTime(to.at.0.saturating_sub(transition.duration.0 / 2));
        let end = start
            .checked_add(transition.duration)
            .map_err(|error| error.to_string())?;
        for id in [from.id, to.id] {
            if let Some(span) = native.get_mut(&id) {
                span.start = span.start.min(start);
                span.end = span.end.max(end);
            }
        }
    }
    let mut events = Vec::new();
    for span in native.values() {
        interval(&mut events, *span, true);
    }
    for shot in &project.shots {
        interval(&mut events, shot.span, false);
    }
    events.sort_unstable_by_key(|event| event.0);
    let (mut native, mut shots, mut index) = (0_i32, 0_i32, 0);
    while index < events.len() {
        let at = events[index].0;
        while index < events.len() && events[index].0 == at {
            let count = if events[index].1 {
                &mut native
            } else {
                &mut shots
            };
            *count += events[index].2;
            index += 1;
        }
        if shots > 0 && native == 0 {
            return Err(
                "Camera shots need a 3D replay clip throughout their timeline range.".into(),
            );
        }
    }
    Ok(())
}

/// Equal-time half-open boundaries are applied together so adjacent native clips keep continuous coverage.
fn interval(events: &mut Vec<(MediaTime, bool, i32)>, span: Span, native: bool) {
    events.extend([(span.start, native, 1), (span.end, native, -1)]);
}
