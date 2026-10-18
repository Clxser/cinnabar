//! One rational frame window owns video timestamps and the corresponding clipped PCM sample extent.

use crate::{FrameRate, MediaTime, Span, StudioError, time::MICROS_PER_SECOND};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameRange {
    fps: FrameRate,
    span: Span,
    first: u64,
    end: u64,
}

impl FrameRange {
    /// Includes exactly presentation timestamps inside the requested half-open project range.
    pub fn new(
        fps: FrameRate,
        duration: MediaTime,
        range: Option<Span>,
    ) -> Result<Self, StudioError> {
        fps.validate()?;
        let span = range.unwrap_or(Span::new(MediaTime::ZERO, duration)?);
        span.validate()?;
        if span.end > duration {
            return Err(StudioError::Invalid(
                "export range exceeds the project; reset to Full project or update Export In/Out",
            ));
        }
        let first = frame_at_or_after(fps, span.start)?;
        let end = frame_at_or_after(fps, span.end)?;
        if first >= end {
            return Err(StudioError::Invalid(
                "export range contains no output frames; move Export Out past Export In",
            ));
        }
        Ok(Self {
            fps,
            span,
            first,
            end,
        })
    }

    /// Saved bounds retain their requested project interval even when the output grid changes.
    pub const fn span(self) -> Span {
        self.span
    }

    /// True only when the half-open interval contains an actual exported presentation timestamp.
    pub fn visual_intersects(self, span: Span) -> Result<bool, StudioError> {
        span.validate()?;
        let start = span.start.max(self.span.start);
        let end = span.end.min(self.span.end);
        if start >= end {
            return Ok(false);
        }
        let first = frame_at_or_after(self.fps, start)?.max(self.first);
        Ok(first < self.end && self.fps.frame_time(first)? < end)
    }

    /// Returns global PCM boundaries shared by source admission and the offline mixer.
    pub fn audio_window(
        self,
        span: Span,
        sample_rate: u32,
    ) -> Result<Option<[u64; 2]>, StudioError> {
        span.validate()?;
        let [origin, end, _] = self.audio_bounds(sample_rate)?;
        let sample =
            |at| crate::audio::sample_frame(at, sample_rate).map_err(|_| StudioError::TimeOverflow);
        let start = sample(span.start)?.max(origin);
        let end = sample(span.end)?.min(end);
        Ok((start < end).then_some([start, end]))
    }

    /// Output frame indices always begin at zero even when sampling later project timestamps.
    pub const fn frames(self) -> u64 {
        self.end - self.first
    }

    /// Each global timestamp is computed from its integer source-grid index, never by adding rounded frame steps.
    pub fn time(self, index: u64) -> Result<MediaTime, StudioError> {
        if index >= self.frames() {
            return Err(StudioError::Invalid(
                "export frame is outside its admitted range",
            ));
        }
        self.fps.frame_time(self.first + index)
    }

    /// PCM uses the same rational origin; the sub-sample final pad stays silent rather than reading beyond Out.
    pub fn audio_bounds(self, sample_rate: u32) -> Result<[u64; 3], StudioError> {
        if sample_rate == 0 || sample_rate > crate::audio::MAX_SAMPLE_RATE {
            return Err(StudioError::Invalid("unsupported export audio sample rate"));
        }
        let frame_sample = |frame: u64| -> Result<u64, StudioError> {
            let numerator =
                u128::from(frame) * u128::from(self.fps.denominator) * u128::from(sample_rate);
            u64::try_from(numerator.div_ceil(u128::from(self.fps.numerator)))
                .map_err(|_| StudioError::TimeOverflow)
        };
        let origin = frame_sample(self.first)?;
        let samples = frame_sample(self.frames())?;
        let rational_end = frame_sample(self.end)?;
        let requested_end = if self
            .fps
            .frame_time(self.end)
            .is_ok_and(|at| at == self.span.end)
        {
            rational_end
        } else {
            u64::try_from(
                (u128::from(self.span.end.0) * u128::from(sample_rate))
                    .div_ceil(u128::from(MICROS_PER_SECOND)),
            )
            .map_err(|_| StudioError::TimeOverflow)?
        };
        let end = rational_end.min(requested_end);
        Ok([origin, end, samples])
    }
}

/// Canonical microsecond frame timestamps round down; exact boundary round trips must not skip a frame.
fn frame_at_or_after(fps: FrameRate, at: MediaTime) -> Result<u64, StudioError> {
    let frame = fps.frame_at(at)?;
    Ok(frame + u64::from(fps.frame_time(frame)? < at))
}

/// Export source/device checks use the same transition interval as playback, including both dissolve handles.
pub fn visual_coverage(
    project: &crate::Project,
) -> Result<std::collections::BTreeMap<crate::Id, Span>, StudioError> {
    use crate::{model::TransitionKind, tracks::Track};
    let clips: std::collections::BTreeMap<_, _> =
        project.clips.iter().map(|clip| (clip.id, clip)).collect();
    let mut coverage = std::collections::BTreeMap::new();
    for clip in project
        .clips
        .iter()
        .filter(|clip| clip.enabled && !project.muted_tracks.contains(&Track::Video(clip.track)))
    {
        coverage.insert(clip.id, clip.span()?);
    }
    for transition in project
        .transitions
        .iter()
        .filter(|transition| transition.kind == TransitionKind::Crossfade)
    {
        if !coverage.contains_key(&transition.from) || !coverage.contains_key(&transition.to) {
            continue;
        }
        let to = clips.get(&transition.to).ok_or(StudioError::Missing {
            kind: "clip",
            id: transition.to.0,
        })?;
        let handles = transition.span_at(to.at)?;
        for id in [transition.from, transition.to] {
            let span = coverage.get_mut(&id).expect("admitted transition member");
            span.start = span.start.min(handles.start);
            span.end = span.end.max(handles.end);
        }
    }
    Ok(coverage)
}

#[cfg(test)]
mod tests;
