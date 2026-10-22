//! Individual edits applied to a private project draft before whole-project validation.

use super::*;
use crate::time::MAX_MEDIA_MICROS;

mod linked;

/// Applies only metadata changes; no command starts a recording or writes files.
pub(super) fn apply(project: &mut Project, command: &Command) -> Result<(), StudioError> {
    match command {
        Command::SetTrackMuted { track, muted } => {
            if !track.valid() {
                return Err(StudioError::Invalid("invalid timeline track"));
            }
            if *muted {
                project.muted_tracks.insert(*track);
            } else {
                project.muted_tracks.remove(track);
            }
        }
        Command::Rename { name } => project.name.clone_from(name),
        Command::AddSource { source } => project.sources.push(source.clone()),
        Command::ReplaceSource { source } => replace(
            &mut project.sources,
            source.id,
            |value| value.id,
            source.clone(),
            "media source",
        )?,
        Command::AddClip { clip } => project.clips.push(clip.clone()),
        Command::ReplaceClip { clip } => {
            *clip_mut(project, clip.id)? = clip.clone();
            linked::synchronize(project, clip.id)?;
        }
        Command::Delete { id } => delete(project, *id)?,
        Command::MoveClip { id, at, track } => {
            let clip = clip_mut(project, *id)?;
            clip.at = *at;
            clip.track = *track;
            linked::synchronize(project, *id)?;
        }
        Command::TrimClip { id, source, ripple } => {
            source.validate()?;
            let clip = clip_mut(project, *id)?;
            let before = clip.span()?.end;
            clip.source_range = *source;
            let after = clip.span()?.end;
            linked::synchronize(project, *id)?;
            if *ripple {
                ripple_after(
                    project,
                    before,
                    i128::from(after.0) - i128::from(before.0),
                    *id,
                )?;
            }
        }
        Command::SplitClip { id, at, right } => split(project, *id, *at, *right)?,
        Command::SetRate { id, rate, ripple } => {
            rate.validate()?;
            let clip = clip_mut(project, *id)?;
            let before = clip.span()?.end;
            clip.rate = *rate;
            let after = clip.span()?.end;
            linked::synchronize(project, *id)?;
            if *ripple {
                ripple_after(
                    project,
                    before,
                    i128::from(after.0) - i128::from(before.0),
                    *id,
                )?;
            }
        }
        Command::SetFraming { id, framing } => clip_mut(project, *id)?.framing = *framing,
        Command::SetSpeedRamp { id, ramp, ripple } => {
            let clip = clip_mut(project, *id)?;
            if let Some(ramp) = ramp {
                ramp.validate(clip.rate)?;
            }
            let before = clip.span()?.end;
            clip.speed_ramp.clone_from(ramp);
            let after = clip.span()?.end;
            linked::synchronize(project, *id)?;
            if *ripple {
                ripple_after(
                    project,
                    before,
                    i128::from(after.0) - i128::from(before.0),
                    *id,
                )?;
            }
        }
        Command::AddShot { shot } => project.shots.push(shot.clone()),
        Command::ReplaceShot { shot } => replace(
            &mut project.shots,
            shot.id,
            |v| v.id,
            shot.clone(),
            "camera",
        )?,
        Command::AddEffect { effect } => project.effects.push(effect.clone()),
        Command::ReplaceEffect { effect } => replace(
            &mut project.effects,
            effect.id,
            |v| v.id,
            effect.clone(),
            "effect",
        )?,
        Command::AddAudio { audio } => project.audio.push(audio.clone()),
        Command::ReplaceAudio { audio } => replace(
            &mut project.audio,
            audio.id,
            |v| v.id,
            audio.clone(),
            "audio",
        )?,
        Command::AddTitle { title } => project.titles.push(title.clone()),
        Command::ReplaceTitle { title } => replace(
            &mut project.titles,
            title.id,
            |v| v.id,
            title.clone(),
            "title",
        )?,
        Command::AddMarker { marker } => project.markers.push(marker.clone()),
        Command::SetBeatMarkers { span, markers } => {
            span.validate()?;
            crate::error::limit(
                "tempo markers",
                markers.len(),
                crate::model::MAX_PROJECT_ITEMS,
            )?;
            if markers.iter().any(|marker| {
                marker.kind != crate::model::MarkerKind::Beat || !span.contains(marker.at)
            }) {
                return Err(StudioError::Invalid(
                    "tempo markers must be beats inside their retained interval",
                ));
            }
            project.markers.retain(|marker| {
                marker.kind != crate::model::MarkerKind::Beat || !span.contains(marker.at)
            });
            project.markers.extend(markers.iter().cloned());
        }
        Command::ReplaceMarker { marker } => replace(
            &mut project.markers,
            marker.id,
            |v| v.id,
            marker.clone(),
            "marker",
        )?,
        Command::AddTransition { transition } => project.transitions.push(transition.clone()),
        Command::SetExport { settings } => project.export = settings.clone(),
        Command::ClearAutomation { source } => {
            project.effects.retain(|effect| {
                !effect
                    .origin
                    .is_some_and(|origin| source.is_none_or(|id| origin.source == id))
            });
            project.markers.retain(|marker| {
                !marker
                    .origin
                    .is_some_and(|origin| source.is_none_or(|id| origin.source == id))
            });
        }
    }
    Ok(())
}

/// Reports stale UI selections rather than silently editing another item.
fn clip_mut(project: &mut Project, id: Id) -> Result<&mut Clip, StudioError> {
    project
        .clips
        .iter_mut()
        .find(|clip| clip.id == id)
        .ok_or(StudioError::Missing {
            kind: "clip",
            id: id.0,
        })
}

/// Replaces one stable-identity item while keeping the surrounding track order intact.
fn replace<T>(
    items: &mut [T],
    id: Id,
    key: impl Fn(&T) -> Id,
    value: T,
    kind: &'static str,
) -> Result<(), StudioError> {
    let item = items
        .iter_mut()
        .find(|item| key(item) == id)
        .ok_or(StudioError::Missing { kind, id: id.0 })?;
    *item = value;
    Ok(())
}

/// Splits an enabled source interval at an output time; source coverage remains contiguous.
fn split(project: &mut Project, id: Id, at: MediaTime, right: Id) -> Result<(), StudioError> {
    if right.0 == 0 || project.ids().any(|existing| existing == right) {
        return Err(StudioError::Invalid("split needs a new nonzero identifier"));
    }
    let clip = clip_mut(project, id)?;
    let span = clip.span()?;
    if at <= span.start || at >= span.end {
        return Err(StudioError::Invalid(
            "a split must be strictly inside the clip",
        ));
    }
    let source = clip.source_time(at)?;
    if source <= clip.source_range.start || source >= clip.source_range.end {
        return Err(StudioError::Invalid(
            "split leaves an empty source interval",
        ));
    }
    let mut next = clip.clone();
    next.id = right;
    next.source_range.start = source;
    clip.source_range.end = source;
    next.at = clip.span()?.end;
    project.clips.push(next);
    linked::split(project, id, right)?;
    // A transition at the old right edge follows the newly created right segment.
    for transition in &mut project.transitions {
        if transition.from == id {
            transition.from = right;
        }
    }
    Ok(())
}

/// Removes an item and dependent transitions; media in use cannot be removed accidentally.
fn delete(project: &mut Project, id: Id) -> Result<(), StudioError> {
    if !project.ids().any(|existing| existing == id) {
        return Err(StudioError::Missing {
            kind: "timeline item",
            id: id.0,
        });
    }
    if project.sources.iter().any(|source| source.id == id)
        && (project.clips.iter().any(|clip| clip.source == id)
            || project.audio.iter().any(|clip| clip.source == id))
    {
        return Err(StudioError::Invalid(
            "remove the source's timeline clips before removing its media",
        ));
    }
    project.sources.retain(|v| v.id != id);
    project.clips.retain(|v| v.id != id);
    project.shots.retain(|v| v.id != id);
    project.effects.retain(|v| v.id != id);
    project
        .audio
        .retain(|v| v.id != id && v.linked_clip != Some(id));
    project.titles.retain(|v| v.id != id);
    project.markers.retain(|v| v.id != id);
    project
        .transitions
        .retain(|v| v.id != id && v.from != id && v.to != id);
    Ok(())
}

/// Moves whole items beginning at the edited boundary; earlier overlapping tracks retain their anchors.
fn ripple_after(
    project: &mut Project,
    boundary: MediaTime,
    delta: i128,
    except: Id,
) -> Result<(), StudioError> {
    for clip in &mut project.clips {
        if clip.id != except && clip.at >= boundary {
            clip.at = shift(clip.at, delta)?;
        }
    }
    for audio in &mut project.audio {
        if audio.at >= boundary {
            audio.at = shift(audio.at, delta)?;
        }
    }
    for shot in &mut project.shots {
        shift_span(&mut shot.span, boundary, delta)?;
    }
    for effect in &mut project.effects {
        if effect.origin.is_none_or(|origin| origin.clip_id.is_none()) {
            shift_span(&mut effect.span, boundary, delta)?;
        }
    }
    for title in &mut project.titles {
        shift_span(&mut title.span, boundary, delta)?;
    }
    for marker in &mut project.markers {
        if marker.at >= boundary && marker.origin.is_none_or(|origin| origin.clip_id.is_none()) {
            marker.at = shift(marker.at, delta)?;
        }
    }
    Ok(())
}

/// Shifts a whole independently anchored clip, preserving its relative keyframe offsets.
fn shift_span(span: &mut Span, boundary: MediaTime, delta: i128) -> Result<(), StudioError> {
    if span.start >= boundary {
        span.start = shift(span.start, delta)?;
        span.end = shift(span.end, delta)?;
    }
    Ok(())
}

/// Rejects a ripple beyond either project bound without lossy signed/unsigned conversions.
fn shift(at: MediaTime, delta: i128) -> Result<MediaTime, StudioError> {
    let next = i128::from(at.0) + delta;
    if !(0..=i128::from(MAX_MEDIA_MICROS)).contains(&next) {
        return Err(StudioError::TimeOverflow);
    }
    Ok(MediaTime(next as u64))
}
