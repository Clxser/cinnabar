//! Group operations preserve relative positions, linked audio and generated source anchors.
use super::*;
use crate::tracks::Track;
use std::collections::{BTreeMap, BTreeSet};

/// Returns editable timeline identities in stable track/time order for range selection.
pub fn selection_ids(project: &Project) -> Vec<Id> {
    let mut items = project
        .clips
        .iter()
        .map(|item| (Track::Video(item.track), item.at, item.id))
        .chain(
            project
                .shots
                .iter()
                .map(|item| (Track::Camera, item.span.start, item.id)),
        )
        .chain(
            project
                .effects
                .iter()
                .map(|item| (Track::Effects, item.span.start, item.id)),
        )
        .chain(
            project
                .audio
                .iter()
                .map(|item| (Track::Audio, item.at, item.id)),
        )
        .chain(
            project
                .titles
                .iter()
                .map(|item| (Track::Titles, item.span.start, item.id)),
        )
        .collect::<Vec<_>>();
    items.sort_unstable();
    items.into_iter().map(|(_, _, id)| id).collect()
}

/// Linked audio and generated effects follow a selected parent without receiving a second shift.
pub(super) fn dependent_owner(project: &Project, id: Id) -> Option<Id> {
    project
        .audio
        .iter()
        .find(|item| item.id == id)
        .and_then(|item| item.linked_clip)
        .or_else(|| {
            project
                .effects
                .iter()
                .find(|item| item.id == id)
                .and_then(|item| item.origin)
                .and_then(|origin| origin.clip_id)
        })
}

/// One delta is clamped against the whole group so no member loses its spacing at either bound.
pub fn move_selection(
    project: &Project,
    selected: &BTreeSet<Id>,
    delta: i128,
) -> Result<(Vec<Command>, i128), StudioError> {
    crate::error::limit(
        "selected items",
        selected.len(),
        crate::edit::MAX_EDIT_COMMANDS,
    )?;
    let roots = selected
        .iter()
        .copied()
        .filter(|id| !dependent_owner(project, *id).is_some_and(|owner| selected.contains(&owner)))
        .collect::<Vec<_>>();
    let spans = roots
        .iter()
        .map(|id| item_span(project, *id))
        .collect::<Result<Vec<_>, _>>()?;
    let start = spans
        .iter()
        .map(|span| span.start.0)
        .min()
        .ok_or(StudioError::Invalid("select timeline items first"))?;
    let end = spans.iter().map(|span| span.end.0).max().unwrap_or(start);
    let delta = delta.clamp(
        -i128::from(start),
        i128::from(crate::time::MAX_MEDIA_MICROS - end),
    );
    let mut commands = Vec::new();
    for id in roots {
        commands.extend(drag(project, id, DragMode::Move, delta, false)?.commands);
    }
    Ok((commands, delta))
}

/// Parent removal owns dependent cleanup; explicitly selected children are never deleted twice.
pub fn delete_selection(
    project: &Project,
    selected: &BTreeSet<Id>,
) -> Result<Vec<Command>, StudioError> {
    let commands = selected
        .iter()
        .filter(|id| {
            !dependent_owner(project, **id).is_some_and(|parent| selected.contains(&parent))
        })
        .map(|id| item_span(project, *id).map(|_| Command::Delete { id: *id }))
        .collect::<Result<Vec<_>, _>>()?;
    if commands.is_empty() {
        return Err(StudioError::Invalid("select timeline items first"));
    }
    Ok(commands)
}

/// Copies the group into its next free interval and remaps copied dependencies to fresh IDs.
pub fn duplicate_selection(
    project: &Project,
    selected: &BTreeSet<Id>,
) -> Result<(Vec<Command>, BTreeSet<Id>), StudioError> {
    crate::error::limit(
        "selected items",
        selected.len(),
        crate::edit::MAX_EDIT_COMMANDS,
    )?;
    let mut included = selected.clone();
    for audio in &project.audio {
        if audio.linked_clip.is_some_and(|id| selected.contains(&id)) {
            included.insert(audio.id);
        }
    }
    for effect in &project.effects {
        if effect
            .origin
            .and_then(|origin| origin.clip_id)
            .is_some_and(|id| selected.contains(&id))
        {
            included.insert(effect.id);
        }
    }
    for marker in &project.markers {
        if marker
            .origin
            .and_then(|origin| origin.clip_id)
            .is_some_and(|id| selected.contains(&id))
        {
            included.insert(marker.id);
        }
    }
    for transition in &project.transitions {
        if selected.contains(&transition.from) && selected.contains(&transition.to) {
            included.insert(transition.id);
        }
    }
    let spans = selected
        .iter()
        .map(|id| item_span(project, *id))
        .collect::<Result<Vec<_>, _>>()?;
    let start = spans
        .iter()
        .map(|span| span.start.0)
        .min()
        .ok_or(StudioError::Invalid("select timeline items first"))?;
    let end = spans.iter().map(|span| span.end.0).max().unwrap_or(start);
    let delta = duplicate_offset(project, selected, i128::from(end - start))?;
    let mut next = project.next_id()?.0;
    let mut ids = BTreeMap::new();
    for id in included {
        ids.insert(id, Id(next));
        next = next
            .checked_add(1)
            .ok_or(StudioError::IdentifiersExhausted)?;
    }
    let mut commands = Vec::new();
    for item in &project.clips {
        if let Some(&id) = ids.get(&item.id) {
            let mut clip = item.clone();
            clip.id = id;
            clip.at = shift(clip.at, delta)?;
            commands.push(Command::AddClip { clip });
        }
    }
    for item in &project.audio {
        if let Some(&id) = ids.get(&item.id) {
            let mut audio = item.clone();
            audio.id = id;
            audio.at = shift(audio.at, delta)?;
            audio.linked_clip = audio.linked_clip.and_then(|old| ids.get(&old).copied());
            commands.push(Command::AddAudio { audio });
        }
    }
    for item in &project.shots {
        if let Some(&id) = ids.get(&item.id) {
            let mut shot = item.clone();
            shot.id = id;
            shot.span = shifted(shot.span, delta)?;
            commands.push(Command::AddShot { shot });
        }
    }
    for item in &project.effects {
        if let Some(&id) = ids.get(&item.id) {
            let mut effect = item.clone();
            effect.id = id;
            effect.span = shifted(effect.span, delta)?;
            if let Some(origin) = &mut effect.origin {
                origin.clip_id = origin.clip_id.and_then(|old| ids.get(&old).copied());
                if origin.clip_id.is_none() {
                    effect.origin = None;
                }
            }
            commands.push(Command::AddEffect { effect });
        }
    }
    for item in &project.titles {
        if let Some(&id) = ids.get(&item.id) {
            let mut title = item.clone();
            title.id = id;
            title.span = shifted(title.span, delta)?;
            commands.push(Command::AddTitle { title });
        }
    }
    for item in &project.markers {
        if let Some(&id) = ids.get(&item.id) {
            let mut marker = item.clone();
            marker.id = id;
            marker.at = shift(marker.at, delta)?;
            if let Some(origin) = &mut marker.origin {
                origin.clip_id = origin.clip_id.and_then(|old| ids.get(&old).copied());
            }
            commands.push(Command::AddMarker { marker });
        }
    }
    for item in &project.transitions {
        if let Some(&id) = ids.get(&item.id) {
            let mut transition = item.clone();
            transition.id = id;
            transition.from = ids[&item.from];
            transition.to = ids[&item.to];
            commands.push(Command::AddTransition { transition });
        }
    }
    crate::error::limit(
        "edit commands",
        commands.len(),
        crate::edit::MAX_EDIT_COMMANDS,
    )?;
    Ok((
        commands,
        selected
            .iter()
            .filter_map(|id| ids.get(id).copied())
            .collect(),
    ))
}

/// A locked logical track forbids direct edits and automatic dependent changes in the same transaction.
pub fn ensure_unlocked(
    before: &Project,
    after: &Project,
    locked: &BTreeSet<Track>,
) -> Result<(), StudioError> {
    for track in locked {
        let same = match *track {
            Track::Video(index) => before
                .clips
                .iter()
                .filter(|clip| clip.track == index)
                .eq(after.clips.iter().filter(|clip| clip.track == index)),
            Track::Camera => before.shots == after.shots,
            Track::Effects => before.effects == after.effects,
            Track::Audio => before.audio == after.audio,
            Track::Titles => before.titles == after.titles,
        };
        if !same {
            return Err(StudioError::Invalid(
                "unlock the affected timeline track before editing",
            ));
        }
    }
    Ok(())
}

/// Shared group offsets keep relative keys inside their unchanged owning interval.
fn shifted(span: Span, delta: i128) -> Result<Span, StudioError> {
    Span::new(shift(span.start, delta)?, shift(span.end, delta)?)
}

/// Monotone obstacle cursors visit each existing clip at most once per selected visual.
fn duplicate_offset(
    project: &Project,
    selected: &BTreeSet<Id>,
    mut delta: i128,
) -> Result<i128, StudioError> {
    let mut obstacles = BTreeMap::<u16, Vec<Span>>::new();
    let mut moving = Vec::new();
    for clip in project.clips.iter().filter(|clip| clip.enabled) {
        let span = clip.span()?;
        obstacles.entry(clip.track).or_default().push(span);
        if selected.contains(&clip.id) {
            moving.push((clip.track, span, 0usize));
        }
    }
    for spans in obstacles.values_mut() {
        spans.sort_by_key(|span| span.start);
    }
    loop {
        let before = delta;
        for (track, original, cursor) in &mut moving {
            let spans = &obstacles[&*track];
            let span = shifted(*original, delta)?;
            while spans
                .get(*cursor)
                .is_some_and(|obstacle| obstacle.end <= span.start)
            {
                *cursor += 1;
            }
            if let Some(obstacle) = spans
                .get(*cursor)
                .filter(|obstacle| span.overlaps(**obstacle))
            {
                delta = delta.max(i128::from(obstacle.end.0) - i128::from(original.start.0));
            }
        }
        if before == delta {
            return Ok(delta);
        }
    }
}
