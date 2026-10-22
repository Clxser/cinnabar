//! Magnetic candidates are compiled once per gesture, independently of paint frequency.
use super::*;
use std::collections::BTreeSet;

pub struct SnapIndex(Vec<MediaTime>);
impl SnapIndex {
    /// Excludes moving items and their dependent audio/evidence from edge candidates.
    pub fn new(project: &Project, selected: &BTreeSet<Id>, playhead: MediaTime) -> Self {
        let mut points = BTreeSet::from([MediaTime::ZERO, playhead]);
        for id in selection_ids(project) {
            if selected.contains(&id)
                || group::dependent_owner(project, id).is_some_and(|id| selected.contains(&id))
            {
                continue;
            }
            if let Ok(span) = item_span(project, id) {
                points.extend([span.start, span.end]);
            }
        }
        points.extend(
            project
                .markers
                .iter()
                .filter(|marker| {
                    !marker
                        .origin
                        .and_then(|origin| origin.clip_id)
                        .is_some_and(|id| selected.contains(&id))
                })
                .map(|marker| marker.at),
        );
        Self(points.into_iter().collect())
    }

    /// One correction aligns either group boundary; equal distances prefer the earlier candidate.
    pub fn delta(
        &self,
        edges: &[MediaTime],
        delta: i128,
        threshold: MediaTime,
    ) -> (i128, Option<MediaTime>) {
        let mut best: Option<(u128, MediaTime, i128)> = None;
        for edge in edges {
            let wanted = i128::from(edge.0) + delta;
            let index = self.0.partition_point(|point| i128::from(point.0) < wanted);
            for point in self.0[index.saturating_sub(1)..(index + 1).min(self.0.len())]
                .iter()
                .copied()
            {
                let correction = i128::from(point.0) - wanted;
                let distance = correction.unsigned_abs();
                if distance <= u128::from(threshold.0)
                    && best.is_none_or(|old| (distance, point) < (old.0, old.1))
                {
                    best = Some((distance, point, correction));
                }
            }
        }
        best.map_or((delta, None), |(_, at, correction)| {
            (delta + correction, Some(at))
        })
    }
}
