//! Whole-rail distance is independent of key spacing; geometric corners receive explicit smooth stops.

use super::*;

const MAX_STOP_HALF_WIDTH: f64 = 0.025;

pub(super) struct GlobalRail {
    source_start: MediaTime,
    source_end: MediaTime,
    easing: Easing,
    cumulative: Vec<f64>,
    stops: Vec<StopZone>,
}

struct StopZone {
    center: f64,
    half_width: f64,
}

impl GlobalRail {
    /// Length prefixes and discontinuous join directions are compiled only with the immutable rail.
    pub(super) fn new(segments: &[ArcSegment], easing: Easing) -> Self {
        let mut cumulative = Vec::with_capacity(segments.len() + 1);
        cumulative.push(0.0);
        for segment in segments {
            cumulative.push(cumulative.last().copied().unwrap_or(0.0) + segment.length());
        }
        let total = cumulative.last().copied().unwrap_or(0.0);
        let mut corners = Vec::new();
        let mut previous: Option<&ArcSegment> = None;
        if total > 0.0 {
            for (index, segment) in segments
                .iter()
                .enumerate()
                .filter(|(_, segment)| segment.length() > 0.0)
            {
                if let Some(before) = previous {
                    let incoming = endpoint_direction(before.controls, false);
                    let outgoing = endpoint_direction(segment.controls, true);
                    if incoming.dot(outgoing) < 1.0 - 1e-8 {
                        let at = cumulative[index] / total;
                        if at > 0.0 && at < 1.0 && corners.last().copied() != Some(at) {
                            corners.push(at);
                        }
                    }
                }
                previous = Some(segment);
            }
        }
        let stops = corners
            .iter()
            .copied()
            .enumerate()
            .map(|(index, center)| {
                let before = index.checked_sub(1).map_or(0.0, |index| corners[index]);
                let after = corners.get(index + 1).copied().unwrap_or(1.0);
                StopZone {
                    center,
                    half_width: MAX_STOP_HALF_WIDTH
                        .min((center - before).min(after - center) * 0.25),
                }
            })
            .collect();
        Self {
            source_start: segments
                .first()
                .map_or(MediaTime::ZERO, |segment| segment.from.at),
            source_end: segments
                .last()
                .map_or(MediaTime::ZERO, |segment| segment.to.at),
            easing,
            cumulative,
            stops,
        }
    }

    /// Global acceleration uses the original path source clock, so trims do not restart an ease.
    pub(super) fn distance_at(&self, source: MediaTime) -> f64 {
        let duration = self.source_end.0.saturating_sub(self.source_start.0);
        if duration == 0 {
            return 0.0;
        }
        let amount =
            (source.0.saturating_sub(self.source_start.0) as f64 / duration as f64).clamp(0.0, 1.0);
        let progress = self.easing.sample(amount);
        let zone = self
            .stops
            .partition_point(|zone| zone.center + zone.half_width <= progress);
        let progress = self
            .stops
            .get(zone)
            .filter(|zone| (progress - zone.center).abs() < zone.half_width)
            .map_or(progress, |zone| zone.sample(progress));
        progress * self.cumulative.last().copied().unwrap_or(0.0)
    }

    /// The half-open segment range intersects traveled distance, independently of orientation timestamps.
    pub(super) fn segment_range(&self, source: [MediaTime; 2]) -> std::ops::Range<usize> {
        let low = self.distance_at(source[0]);
        let high = self.distance_at(source[1]);
        if high <= low {
            return 0..0;
        }
        let count = self.cumulative.len().saturating_sub(1);
        let start = self.segment_at_distance(low);
        let end = self
            .cumulative
            .partition_point(|prefix| *prefix < high)
            .min(count);
        start..end
    }

    /// An exact boundary selects the outgoing segment, skipping stationary distance plateaus.
    pub(super) fn segment_at(&self, source: MediaTime) -> usize {
        self.segment_at_distance(self.distance_at(source))
    }

    /// Prefix search is shared by playback and overlay admission so they cannot choose different rails.
    fn segment_at_distance(&self, distance: f64) -> usize {
        self.cumulative
            .partition_point(|prefix| *prefix <= distance)
            .saturating_sub(1)
            .min(self.cumulative.len().saturating_sub(2))
    }

    /// Empty-distance intervals are skipped without losing independent orientation keys or producing NaNs.
    pub(super) fn position(&self, segments: &[ArcSegment], source: MediaTime) -> [f64; 3] {
        let distance = self.distance_at(source);
        let total = self.cumulative.last().copied().unwrap_or(0.0);
        if distance <= 0.0 {
            return segments
                .first()
                .map_or([0.0; 3], |segment| segment.from.pose.position);
        }
        if distance >= total {
            return segments
                .last()
                .map_or([0.0; 3], |segment| segment.to.pose.position);
        }
        let index = self.segment_at_distance(distance);
        let segment = &segments[index];
        let length = segment.length();
        let fraction = if length > 0.0 {
            (distance - self.cumulative[index]) / length
        } else {
            0.0
        };
        cubic(segment.controls, segment.parameter(fraction)).to_array()
    }

    /// Corners have exactly zero travel rate; smooth joins retain the same normalized spatial derivative.
    pub(super) fn velocity(&self, segments: &[ArcSegment], source: MediaTime) -> DVec3 {
        if source < self.source_start || source > self.source_end {
            return DVec3::ZERO;
        }
        let duration = self.source_end.saturating_sub(self.source_start).seconds();
        if duration <= 0.0 {
            return DVec3::ZERO;
        }
        let amount = source.saturating_sub(self.source_start).seconds() / duration;
        let progress = self.easing.sample(amount);
        let zone = self
            .stops
            .partition_point(|zone| zone.center + zone.half_width <= progress);
        let slope = self
            .stops
            .get(zone)
            .filter(|zone| (progress - zone.center).abs() < zone.half_width)
            .map_or(1.0, |zone| zone.derivative(progress));
        let total = self.cumulative.last().copied().unwrap_or(0.0);
        let speed = self.easing.derivative(amount) * slope * total / duration;
        if speed == 0.0 {
            return DVec3::ZERO;
        }
        let distance = self.distance_at(source);
        let index = self.segment_at_distance(distance);
        let segment = &segments[index];
        let fraction = if segment.length() > 0.0 {
            (distance - self.cumulative[index]) / segment.length()
        } else {
            0.0
        };
        segment.direction(segment.parameter(fraction)) * speed
    }

    /// Clipping a debug path overlay uses the same traveled distances as rendered playback.
    pub(super) fn local_amounts(&self, index: usize, source: [MediaTime; 2]) -> Option<[f64; 2]> {
        let start = *self.cumulative.get(index)?;
        let end = *self.cumulative.get(index + 1)?;
        if end <= start {
            return None;
        }
        let low = self.distance_at(source[0]).max(start);
        let high = self.distance_at(source[1]).min(end);
        (high > low).then(|| {
            [
                (low - start) / (end - start),
                (high - start) / (end - start),
            ]
        })
    }
}

impl StopZone {
    /// This quintic has zero velocity/acceleration at a corner and unit velocity/zero acceleration at its edges.
    fn sample(&self, progress: f64) -> f64 {
        let offset = (progress - self.center) / self.half_width;
        let x = offset.abs().clamp(0.0, 1.0);
        let distance = x * x * x * (6.0 + x * (-8.0 + 3.0 * x));
        self.center + offset.signum() * self.half_width * distance
    }

    /// The derivative uses the same quintic coefficients factored to retain exact corner and edge values.
    fn derivative(&self, progress: f64) -> f64 {
        let x = ((progress - self.center) / self.half_width)
            .abs()
            .clamp(0.0, 1.0);
        x * x * (18.0 + x * (-32.0 + 15.0 * x))
    }
}

/// The first nonzero endpoint derivative term supplies its one-sided limiting travel direction.
pub(super) fn endpoint_direction(points: [DVec3; 4], start: bool) -> DVec3 {
    let candidates = if start {
        [
            points[1] - points[0],
            points[2] - points[0],
            points[3] - points[0],
        ]
    } else {
        [
            points[3] - points[2],
            points[3] - points[1],
            points[3] - points[0],
        ]
    };
    candidates
        .into_iter()
        .find_map(integration::normalized)
        .unwrap_or(DVec3::ZERO)
}

#[cfg(test)]
mod tests;
