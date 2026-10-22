//! Camera admission uses verified source intervals and the clip's exact monotone retiming map.

use std::path::PathBuf;

use super::CameraTargetRef;
use crate::{MediaTime, Span, StudioError, export::Cancellation, retime::TimeMap};

mod worker;
pub use worker::TargetVerifier;

#[derive(Clone, Debug, PartialEq)]
pub struct TargetCoverageRequest {
    pub directory: PathBuf,
    pub source_range: Span,
    pub target: CameraTargetRef,
    pub require_eye: bool,
}

/// A continuous half-open interval belongs to one admitted spawn lifetime, never only its runtime ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TargetCoverage {
    pub source_range: Span,
    pub target: CameraTargetRef,
}

/// The scene owner supplies disk verification; UI callers only transfer this callback to an IO worker.
pub type TargetCoverageReader = fn(&TargetCoverageRequest, &Cancellation) -> TargetCoverageResult;

/// Missing subjects are ordinary coverage gaps; malformed or mismatched sources are explicit errors.
pub type TargetCoverageResult = Result<Option<TargetCoverage>, String>;

/// Every source microsecond reachable by a positive-rate shot is covered, including transition handles.
pub fn source_window(
    map: &TimeMap,
    timeline: Span,
    duration: MediaTime,
) -> Result<Span, StudioError> {
    timeline.validate()?;
    let first = map.transition_time(timeline.start, duration)?;
    let last = map.transition_time(MediaTime(timeline.end.0 - 1), duration)?;
    Span::new(first, last.checked_add(MediaTime(1))?)
}

/// Finds the first timeline microsecond that reaches unverified source time without floating-point inversion.
pub fn timeline_prefix(
    map: &TimeMap,
    timeline: Span,
    duration: MediaTime,
    coverage: TargetCoverage,
) -> Result<Option<Span>, StudioError> {
    timeline.validate()?;
    if map.transition_time(timeline.start, duration)? < coverage.source_range.start {
        return Ok(None);
    }
    let (mut low, mut high) = (timeline.start.0, timeline.end.0);
    while low < high {
        let middle = low + (high - low) / 2;
        if map.transition_time(MediaTime(middle), duration)? < coverage.source_range.end {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    Ok((low > timeline.start.0).then_some(Span {
        start: timeline.start,
        end: MediaTime(low),
    }))
}

#[cfg(test)]
mod tests;
