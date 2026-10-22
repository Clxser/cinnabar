//! Conservative annotations read the complete committed primary stream, including older checkpoints.

use crate::{
    MediaTime,
    archive::{ArchiveError, RecordKind, RecordingReader},
    automation::{CombatDetector, GameplayEvent, Highlight},
    export::Cancellation,
};
use std::path::Path;

pub(super) struct Events {
    pub highlights: Vec<Highlight>,
    pub omitted: u64,
    pub observed: u64,
}

/// Legacy observation gaps reset only RAM state, so every event is interpreted without cross-event correlations.
pub(super) fn read(
    parent: &Path,
    prefix: MediaTime,
    limit: usize,
    cancellation: &Cancellation,
) -> Result<Events, String> {
    cancellation.check()?;
    let reader = RecordingReader::open(parent).map_err(|error| error.to_string())?;
    if reader.info().scene_schema != super::super::POV_SCENE_SCHEMA {
        return Err("Unsupported recorded event schema.".into());
    }
    let local_actor = reader.info().local_actor;
    let mut cursor = reader.into_cursor();
    let mut events = Events {
        highlights: Vec::new(),
        omitted: 0,
        observed: 0,
    };
    let mut sequence = 0_u64;
    let mut detector = CombatDetector::new(local_actor);
    let mut scratch = Vec::new();
    let mut visit = |record: crate::archive::ReplayRecord<'_>| {
        if cancellation.cancelled() {
            return Err(ArchiveError::Cancelled);
        }
        if record.kind != RecordKind::Gameplay {
            return Ok(());
        }
        let event: GameplayEvent = serde_json::from_slice(record.payload)?;
        if event.at != record.at || sequence.checked_add(1) != Some(event.sequence) {
            return Err(ArchiveError::Invalid(
                "Recorded event timestamps or sequences are inconsistent.",
            ));
        }
        sequence = event.sequence;
        scratch.clear();
        detector
            .observe_isolated(&event, &mut scratch)
            .map_err(|_| ArchiveError::Invalid("Invalid recorded gameplay event."))?;
        events.observed += 1;
        if event.at >= prefix {
            return Ok(());
        }
        for event in scratch.drain(..) {
            if events.highlights.len() < limit {
                events.highlights.push(event);
            } else {
                events.omitted += 1;
            }
        }
        Ok(())
    };
    cursor
        .seek(MediaTime::ZERO, || cancellation.cancelled(), &mut visit)
        .map_err(|error| error.to_string())?;
    cursor
        .advance_to(prefix, || cancellation.cancelled(), &mut visit)
        .map_err(|error| error.to_string())?;
    Ok(events)
}

#[cfg(test)]
mod tests;
