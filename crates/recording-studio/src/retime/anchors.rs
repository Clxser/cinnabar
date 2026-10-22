//! Generated metadata follows its recorded source event through complete clip edit transactions.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    Id, Project,
    effect::{AutomationOrigin, Curve, CurveKey, EffectClip},
    model::Clip,
};

use super::*;

/// Reconciles explicit source anchors once per transaction; legacy origins without anchors stay untouched.
pub(crate) fn synchronize(before: &Project, after: &mut Project) -> Result<(), StudioError> {
    let old = before
        .clips
        .iter()
        .map(|clip| (clip.id, clip))
        .collect::<BTreeMap<_, _>>();
    let new = after
        .clips
        .iter()
        .map(|clip| (clip.id, clip))
        .collect::<BTreeMap<_, _>>();
    if old.len() == new.len()
        && old
            .iter()
            .all(|(id, clip)| new.get(id).is_some_and(|next| same_clock(clip, next)))
    {
        return Ok(());
    }
    let old_maps = old
        .iter()
        .map(|(&id, clip)| clip.time_map().map(|map| (id, map)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let new_maps = new
        .iter()
        .map(|(&id, clip)| clip.time_map().map(|map| (id, map)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let old_effects = before
        .effects
        .iter()
        .map(|effect| effect.id)
        .collect::<BTreeSet<_>>();
    let old_markers = before
        .markers
        .iter()
        .map(|marker| marker.id)
        .collect::<BTreeSet<_>>();
    let mut effects = Vec::with_capacity(after.effects.len());
    for mut effect in after.effects.drain(..) {
        let Some(origin) = effect.origin.filter(|_| old_effects.contains(&effect.id)) else {
            effects.push(effect);
            continue;
        };
        let (Some(id), Some(source)) = (origin.clip_id, origin.source_time) else {
            effects.push(effect);
            continue;
        };
        let Some(previous) = old.get(&id) else {
            effects.push(effect);
            continue;
        };
        if new.get(&id).is_some_and(|next| same_clock(previous, next)) {
            effects.push(effect);
            continue;
        }
        let Some(owner) = owner(origin, previous, &old, &new) else {
            continue;
        };
        if remap_effect(
            &mut effect,
            origin,
            &old_maps[&id],
            &new_maps[&owner.id],
            owner,
            source,
        )? {
            effects.push(effect);
        }
    }
    after.effects = effects;
    let mut markers = Vec::with_capacity(after.markers.len());
    for mut marker in after.markers.drain(..) {
        let Some(mut origin) = marker.origin.filter(|_| old_markers.contains(&marker.id)) else {
            markers.push(marker);
            continue;
        };
        let (Some(id), Some(source)) = (origin.clip_id, origin.source_time) else {
            markers.push(marker);
            continue;
        };
        let Some(previous) = old.get(&id) else {
            markers.push(marker);
            continue;
        };
        if new.get(&id).is_some_and(|next| same_clock(previous, next)) {
            markers.push(marker);
            continue;
        }
        let Some(owner) = owner(origin, previous, &old, &new) else {
            continue;
        };
        let Some(at) = new_maps[&owner.id].timeline_time(source)? else {
            continue;
        };
        marker.at = at;
        origin.clip_id = Some(owner.id);
        marker.origin = Some(origin);
        markers.push(marker);
    }
    after.markers = markers;
    Ok(())
}

/// Source evidence belongs to its retained clip or a new split segment, never an unrelated repeated take.
fn owner<'a>(
    origin: AutomationOrigin,
    previous: &Clip,
    old: &BTreeMap<Id, &Clip>,
    new: &'a BTreeMap<Id, &'a Clip>,
) -> Option<&'a Clip> {
    let source = origin.source_time?;
    let id = origin.clip_id?;
    if let Some(clip) = new
        .get(&id)
        .filter(|clip| clip.source == origin.source && clip.source_range.contains(source))
    {
        return Some(clip);
    }
    new.values()
        .copied()
        .filter(|clip| {
            !old.contains_key(&clip.id)
                && clip.source == origin.source
                && clip.source_range.contains(source)
                && clip.source_range.start >= previous.source_range.start
                && clip.source_range.end <= previous.source_range.end
        })
        .min_by_key(|clip| (clip.at, clip.id))
}

/// Only source-clock changes retime generated metadata; crop, track and opacity edits preserve it.
fn same_clock(a: &Clip, b: &Clip) -> bool {
    a.source == b.source
        && a.source_range == b.source_range
        && a.at == b.at
        && a.rate == b.rate
        && a.speed_ramp == b.speed_ramp
}

/// Key poses retain their easing and values while source positions and the observed peak follow the new map.
fn remap_effect(
    effect: &mut EffectClip,
    mut origin: AutomationOrigin,
    old: &TimeMap,
    new: &TimeMap,
    owner: &Clip,
    event: MediaTime,
) -> Result<bool, StudioError> {
    effect.span.validate()?;
    effect.amount.validate(effect.span.duration())?;
    let start = old
        .source_boundary(effect.span.start)?
        .max(owner.source_range.start);
    let end = old
        .source_boundary(effect.span.end)?
        .min(owner.source_range.end);
    if start >= end {
        return Ok(false);
    }
    let span = match Span::new(new.timeline_boundary(start)?, new.timeline_boundary(end)?) {
        Ok(span) => span,
        Err(_) => return Ok(false),
    };
    let old_event = old.timeline_time(event)?;
    let mut keys = BTreeMap::new();
    let value_at = |source| -> Result<f64, StudioError> {
        let at = old
            .timeline_boundary(source)?
            .saturating_sub(effect.span.start);
        Ok(effect.amount.sample(at))
    };
    keys.insert(
        MediaTime::ZERO,
        CurveKey {
            at: MediaTime::ZERO,
            value: value_at(start)?,
            easing: Easing::Linear,
        },
    );
    for key in &effect.amount.keys {
        let absolute = effect.span.start.checked_add(key.at)?;
        let source = if Some(absolute) == old_event {
            event
        } else {
            old.source_boundary(absolute)?
        };
        if source <= start || source >= end {
            continue;
        }
        let mapped = if Some(absolute) == old_event {
            let Some(at) = new.timeline_time(event)? else {
                continue;
            };
            at
        } else {
            new.timeline_boundary(source)?
        };
        let at = mapped.saturating_sub(span.start);
        if at < span.duration() {
            keys.insert(
                at,
                CurveKey {
                    at,
                    value: key.value,
                    easing: key.easing,
                },
            );
        }
    }
    let until = old
        .timeline_boundary(end)?
        .saturating_sub(effect.span.start);
    let incoming = effect
        .amount
        .keys
        .partition_point(|key| key.at < until)
        .min(effect.amount.keys.len() - 1);
    keys.insert(
        span.duration(),
        CurveKey {
            at: span.duration(),
            value: value_at(end)?,
            easing: effect.amount.keys[incoming].easing,
        },
    );
    effect.span = span;
    effect.amount = Curve {
        keys: keys.into_values().collect(),
    };
    origin.clip_id = Some(owner.id);
    effect.origin = Some(origin);
    Ok(true)
}
