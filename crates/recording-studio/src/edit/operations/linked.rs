//! Game audio shares a visual edit's time mapping while retaining its own mix controls.

use super::*;

/// Linked audio uses the same recorded source clock; independent tracks stay untouched.
pub(super) fn synchronize(project: &mut Project, id: Id) -> Result<(), StudioError> {
    let visual = project
        .clips
        .iter()
        .find(|clip| clip.id == id)
        .ok_or(StudioError::Missing {
            kind: "clip",
            id: id.0,
        })?;
    let duration = visual.span()?.duration();
    for audio in &mut project.audio {
        if audio.linked_clip == Some(id) {
            audio.at = visual.at;
            audio.source_range = visual.source_range;
            audio.rate = visual.rate;
            audio.fade_in = audio.fade_in.min(duration);
            audio.fade_out = audio.fade_out.min(duration);
        }
    }
    Ok(())
}

/// A visual split creates a contiguous audio split with fresh stable IDs and preserved gain/mute.
pub(super) fn split(project: &mut Project, left: Id, right: Id) -> Result<(), StudioError> {
    if !project
        .audio
        .iter()
        .any(|audio| audio.linked_clip == Some(left))
    {
        return Ok(());
    }
    let visual = project
        .clips
        .iter()
        .find(|clip| clip.id == right)
        .ok_or(StudioError::Missing {
            kind: "clip",
            id: right.0,
        })?
        .clone();
    let mut next = project.next_id()?.0;
    let mut additions = Vec::new();
    for audio in &project.audio {
        if audio.linked_clip != Some(left) {
            continue;
        }
        let mut second = audio.clone();
        second.id = Id(next);
        next = next.checked_add(1).ok_or(StudioError::TimeOverflow)?;
        second.linked_clip = Some(right);
        second.at = visual.at;
        second.source_range = visual.source_range;
        second.rate = visual.rate;
        second.fade_in = MediaTime::ZERO;
        additions.push(second);
    }
    for audio in &mut project.audio {
        if audio.linked_clip == Some(left) {
            audio.fade_out = MediaTime::ZERO;
        }
    }
    project.audio.extend(additions);
    synchronize(project, left)?;
    synchronize(project, right)
}
