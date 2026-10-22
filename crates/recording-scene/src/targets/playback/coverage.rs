//! Coverage follows actual target change boundaries instead of sampling a few guessed future frames.

use super::*;
use recording_studio::{
    Span,
    camera::coverage::{TargetCoverage, TargetCoverageRequest},
};

impl TargetRecording {
    /// Returns only the first continuous valid interval; a later respawn never repairs a prior gap.
    pub(crate) fn continuous_coverage(
        &mut self,
        request: &TargetCoverageRequest,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<TargetCoverage>, ArchiveError> {
        if cancelled() {
            self.invalidate();
            return Err(ArchiveError::Cancelled);
        }
        request
            .source_range
            .validate()
            .map_err(|_| ArchiveError::Invalid("invalid target coverage range"))?;
        let start = request.source_range.start;
        let mut end = request.source_range.end.min(self.duration());
        if start >= end {
            return Ok(None);
        }
        let frame = self.seek(start, cancelled)?;
        let Some(initial) = frame
            .target(request.target.runtime_id)
            .filter(|target| target.matches(request.target))
            .filter(|target| !request.require_eye || target.pose.eye_offset.is_some())
        else {
            return Ok(None);
        };
        let reference = initial.reference();
        loop {
            if cancelled() {
                self.invalidate();
                return Err(ArchiveError::Cancelled);
            }
            let Some(next) = &self.next else {
                break;
            };
            let next_at = next.at;
            if request.require_eye {
                let current = self
                    .current
                    .as_ref()
                    .and_then(|frame| frame.target(reference.runtime_id));
                let future = next.target(reference.runtime_id);
                if let (Some(current), Some(future)) = (current, future)
                    && current.can_interpolate(future)
                    && current.pose.eye_offset.is_some()
                    && future.pose.eye_offset.is_none()
                {
                    let unavailable = self
                        .current
                        .as_ref()
                        .expect("current target admitted")
                        .at
                        .checked_add(MediaTime(1))
                        .map_err(|_| ArchiveError::Invalid("target eye boundary overflows"))?;
                    end = end.min(unavailable);
                    break;
                }
            }
            if next_at >= end {
                break;
            }
            let frame = self.advance_to(next_at, cancelled)?;
            if !frame.target(reference.runtime_id).is_some_and(|target| {
                target.matches(reference)
                    && (!request.require_eye || target.pose.eye_offset.is_some())
            }) {
                end = next_at;
                break;
            }
        }
        if cancelled() {
            self.invalidate();
            return Err(ArchiveError::Cancelled);
        }
        Ok((end > start).then_some(TargetCoverage {
            source_range: Span { start, end },
            target: reference,
        }))
    }
}

#[cfg(test)]
mod tests;
