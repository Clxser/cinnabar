//! Interrupted native companion data is recovered independently without weakening scene verification.

use crate::{
    Project,
    edit::Editor,
    export::Cancellation,
    model::{MAX_PROJECT_ITEMS, MediaKind},
    wav::recovery::PreparedAudio,
};
use std::path::Path;

mod events;

/// The unsaved project and its explicit companion coverage report are published together.
pub struct RecoveredCapture {
    pub editor: Editor,
    pub notice: String,
}

/// Only a verified native project enters this worker operation; failed companions are explained explicitly.
pub fn recover(
    mut project: Project,
    parent: &Path,
    requested_audio: bool,
    cancellation: &Cancellation,
) -> Result<RecoveredCapture, String> {
    cancellation.check()?;
    project.validate().map_err(|error| error.to_string())?;
    let clip = project
        .clips
        .first()
        .ok_or("Recovered project has no native clip.")?
        .clone();
    let source = project
        .sources
        .iter()
        .find(|source| source.id == clip.source && source.kind == MediaKind::Replay)
        .ok_or("Companion recovery requires a verified native source.")?;
    let prefix = source.duration;
    if source.path != parent
        || clip.at != crate::MediaTime::ZERO
        || clip.source_range.start != crate::MediaTime::ZERO
        || clip.source_range.end != prefix
        || clip.rate != Default::default()
        || clip.speed_ramp.is_some()
    {
        return Err("Companion recovery requires the initial verified native interval without edits or retiming.".into());
    }
    let mut notices = Vec::new();
    let mut audio: Option<(PreparedAudio, crate::Id, String)> = None;
    if requested_audio {
        match crate::wav::recovery::prepare(&parent.join(super::AUDIO_FILE), prefix, cancellation) {
            Ok(prepared) => {
                let id = super::project::game_audio(
                    &mut project,
                    clip.id,
                    prepared.path().to_path_buf(),
                    prepared.duration,
                )
                .map_err(|error| error.to_string())?;
                let notice = format!(
                    "Recovered {} physical PCM frames at {} Hz ({:.6}s).{}{} {} bytes outside the retained complete-frame interval were excluded.{}",
                    prepared.frames,
                    prepared.rate,
                    prepared.duration.seconds(),
                    if prepared.unfinished {
                        " Its unfinished RF64 header was finalized in a separate file."
                    } else {
                        ""
                    },
                    if prepared.truncated {
                        " Its declared PCM tail is missing; only samples physically present were retained."
                    } else {
                        ""
                    },
                    prepared.discarded_bytes,
                    if prepared.duration < prefix {
                        " Audio ends before the verified scene interval and remains an unlinked Game audio clip; no missing samples were invented."
                    } else {
                        ""
                    }
                );
                audio = Some((prepared, id, notice));
            }
            Err(error) => {
                cancellation.check()?;
                notices.push(format!(
                    "Game audio was not recovered: {error}. The original sidecar is unchanged."
                ));
            }
        }
    } else {
        notices.push("Game audio was disabled for this take.".into());
    }
    let limit = MAX_PROJECT_ITEMS.saturating_sub(project.ids().count());
    match events::read(parent, prefix, limit, cancellation) {
        Ok(events) => {
            let count = events.highlights.len();
            let mut id = project.next_id().map_err(|error| error.to_string())?.0;
            for event in events.highlights {
                project.markers.push(crate::automation::source_marker(
                    crate::Id(id),
                    clip.source,
                    clip.id,
                    event.at,
                    &event,
                ));
                id = id
                    .checked_add(1)
                    .ok_or("Recovered marker identities are exhausted.")?;
            }
            notices.push(format!("Recovered {count} durable bookmarks or unambiguous observations from {} committed events; {} markers exceeded the project item budget. Legacy combat correlations and combos were not reconstructed because capture-time gap resets were not persisted.", events.observed, events.omitted));
        }
        Err(error) => {
            cancellation.check()?;
            notices.push(format!(
                "Event markers were not recovered: {error}. The original journal is unchanged."
            ));
        }
    }
    // Project admission happens before media publication, including item and serialized byte budgets.
    Editor::recovered(project.clone()).map_err(|error| error.to_string())?;
    cancellation.check()?;
    let published = if let Some((prepared, id, notice)) = audio {
        match prepared.publish(parent, cancellation) {
            Ok(published) => {
                project
                    .sources
                    .iter_mut()
                    .find(|source| source.id == id)
                    .expect("admitted audio source")
                    .path = published.path.clone();
                notices.push(notice);
                Some(published)
            }
            Err(error) => {
                cancellation.check()?;
                project.audio.retain(|audio| audio.source != id);
                project.sources.retain(|source| source.id != id);
                project.export.include_audio = !project.audio.is_empty();
                notices.push(format!("Recovered audio could not be published: {error}. Native replay was recovered without audio; the original sidecar is unchanged."));
                None
            }
        }
    } else {
        None
    };
    let editor = Editor::recovered(project).map_err(|error| error.to_string())?;
    cancellation.begin_publication()?;
    if let Some(published) = published {
        published.retain();
    }
    notices.push("Capture-time dropped-audio and missing-observation statistics are unavailable without a completed manifest. Original journals and audio remain unchanged.".into());
    Ok(RecoveredCapture {
        editor,
        notice: notices.join("\n"),
    })
}

#[cfg(test)]
mod fixture;
#[cfg(test)]
mod tests;
