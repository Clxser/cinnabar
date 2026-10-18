//! Boundary admission retains device and camera coverage checks inside the selected export range.

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
    let range = recording_studio::export::range::FrameRange::new(
        project.export.fps,
        project.duration(),
        project.export.range,
    )
    .map_err(|error| error.to_string())?;
    let coverage = recording_studio::export::range::visual_coverage(project)
        .map_err(|error| error.to_string())?;
    let mut native: BTreeMap<Id, Span> = BTreeMap::new();
    let clips: BTreeMap<_, _> = project.clips.iter().map(|clip| (clip.id, clip)).collect();
    for clip in project.clips.iter().filter(|clip| {
        coverage
            .get(&clip.id)
            .is_some_and(|span| range.visual_intersects(*span).unwrap_or(false))
    }) {
        if kinds.get(&clip.source) == Some(&MediaKind::Replay) {
            if !has_device {
                return Err("Native replay export needs an initialized graphics device.".into());
            }
            native.insert(
                clip.id,
                *coverage.get(&clip.id).expect("admitted visual coverage"),
            );
        }
    }
    if !has_device
        && (project
            .titles
            .iter()
            .any(|title| range.visual_intersects(title.span).unwrap_or(false))
            || project.effects.iter().any(|effect| {
                effect.enabled && range.visual_intersects(effect.span).unwrap_or(false)
            })
            || project.transitions.iter().any(|transition| {
                transition.kind == TransitionKind::DipToBlack
                    && clips.get(&transition.to).is_some_and(|to| {
                        transition
                            .span_at(to.at)
                            .is_ok_and(|span| range.visual_intersects(span).unwrap_or(false))
                    })
            }))
    {
        return Err("Timeline effects and titles need an initialized graphics device.".into());
    }
    let mut events = Vec::new();
    for span in native.values() {
        interval(&mut events, intersection(*span, range.span()), true);
    }
    for shot in project
        .shots
        .iter()
        .filter(|shot| range.visual_intersects(shot.span).unwrap_or(false))
    {
        interval(&mut events, intersection(shot.span, range.span()), false);
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
        if shots > 0
            && native == 0
            && index < events.len()
            && range
                .visual_intersects(Span {
                    start: at,
                    end: events[index].0,
                })
                .map_err(|error| error.to_string())?
        {
            return Err(
                "Camera shots need a 3D replay clip throughout their timeline range.".into(),
            );
        }
    }
    Ok(())
}

/// Only admitted overlapping intervals reach the coverage sweep.
fn intersection(span: Span, range: Span) -> Span {
    Span {
        start: span.start.max(range.start),
        end: span.end.min(range.end),
    }
}

/// Equal-time half-open boundaries are applied together so adjacent native clips keep continuous coverage.
fn interval(events: &mut Vec<(MediaTime, bool, i32)>, span: Span, native: bool) {
    events.extend([(span.start, native, 1), (span.end, native, -1)]);
}

#[cfg(test)]
mod tests;
