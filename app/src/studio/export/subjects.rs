//! Subject-bound shots are verified against native source timelines before any encoder starts.

use recording_studio::{
    Span,
    camera::{
        CameraMode,
        coverage::{self, TargetCoverageRequest},
    },
    export::{
        Cancellation, ExportRequest,
        range::{self, FrameRange},
    },
    model::MediaKind,
    playback::{CompiledTimeline, FramePlan},
};

/// The export worker checks every visible native layer, including retimed dissolve handles.
pub(super) fn prepare(request: &ExportRequest, cancellation: &Cancellation) -> Result<(), String> {
    let project = &request.project;
    let range = request.frame_range()?;
    let coverage = range::visual_coverage(project).map_err(|error| error.to_string())?;
    let mut lifetimes = std::collections::BTreeMap::new();
    for (id, active) in active_camera_ranges(project, cancellation)? {
        let shot = project
            .shots
            .iter()
            .find(|shot| shot.id == id)
            .expect("compiled shot belongs to project");
        if shot.target_reference().is_none() {
            continue;
        }
        for clip in &project.clips {
            cancellation.check()?;
            let Some(clip_span) = coverage.get(&clip.id) else {
                continue;
            };
            let Some(timeline) = sampled_intersection(*clip_span, active, range)? else {
                continue;
            };
            let source = project
                .sources
                .iter()
                .find(|source| source.id == clip.source)
                .ok_or("Camera verification source is missing.")?;
            if source.kind != MediaKind::Replay {
                continue;
            }
            let directory = if source.path.is_absolute() {
                source.path.clone()
            } else {
                request.source_base.join(&source.path)
            };
            let map = clip.time_map().map_err(|error| error.to_string())?;
            let query = TargetCoverageRequest {
                directory,
                source_range: coverage::source_window(&map, timeline, source.duration)
                    .map_err(|error| error.to_string())?,
                target: lifetimes
                    .get(&(shot.id, source.id))
                    .copied()
                    .unwrap_or_else(|| shot.target_reference().expect("subject-bound shot")),
                require_eye: matches!(&shot.mode, CameraMode::Player { .. }),
            };
            let valid = recording_scene::targets::verify_coverage(&query, cancellation)?;
            let Some(valid) = valid.filter(|valid| valid.source_range == query.source_range) else {
                return Err(format!(
                    "Camera shot '{}' outlasts its recorded subject or native eye view in '{}' between {:.3}s and {:.3}s. Shorten or retarget the shot before exporting.",
                    shot.name,
                    source.name,
                    timeline.start.seconds(),
                    timeline.end.seconds(),
                ));
            };
            lifetimes.insert((shot.id, source.id), valid.target);
        }
    }
    cancellation.check()
}

/// Camera precedence and muting come from the playback owner; hidden or superseded shots do not reject exports.
fn active_camera_ranges(
    project: &std::sync::Arc<recording_studio::Project>,
    cancellation: &Cancellation,
) -> Result<Vec<(recording_studio::Id, Span)>, String> {
    let timeline =
        CompiledTimeline::new(std::sync::Arc::clone(project)).map_err(|error| error.to_string())?;
    let mut boundaries: Vec<_> = project
        .shots
        .iter()
        .flat_map(|shot| [shot.span.start, shot.span.end])
        .collect();
    for clip in &project.clips {
        cancellation.check()?;
        let span = clip.span().map_err(|error| error.to_string())?;
        boundaries.extend([span.start, span.end]);
    }
    for transition in &project.transitions {
        let to = project
            .clips
            .iter()
            .find(|clip| clip.id == transition.to)
            .ok_or("Transition source is missing.")?;
        let span = transition
            .span_at(to.at)
            .map_err(|error| error.to_string())?;
        boundaries.extend([span.start, span.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut plan = FramePlan::default();
    let mut active = Vec::new();
    for pair in boundaries.windows(2) {
        cancellation.check()?;
        timeline
            .sample(pair[0], &mut plan, |_, _, _| None)
            .map_err(|error| error.to_string())?;
        if let Some(id) = plan.shot {
            active.push((
                id,
                Span {
                    start: pair[0],
                    end: pair[1],
                },
            ));
        }
    }
    Ok(active)
}

/// Only the first through last actual output timestamps need source coverage in an exported subrange.
fn sampled_intersection(clip: Span, shot: Span, range: FrameRange) -> Result<Option<Span>, String> {
    let start = clip.start.max(shot.start).max(range.span().start);
    let end = clip.end.min(shot.end).min(range.span().end);
    if start >= end {
        return Ok(None);
    }
    range
        .visual_window(Span { start, end })
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
