//! Cached presentation lanes keep overlapping metadata independently selectable without changing render order.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap},
    sync::Arc,
};

use crate::{Id, MediaTime, Project, Span, StudioError};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TimelineLanes {
    pub cameras: Lanes,
    pub effects: Lanes,
    pub audio: Lanes,
    pub titles: Lanes,
    source_names: BTreeMap<Id, Arc<str>>,
    tracks: Arc<[crate::tracks::Track]>,
    markers: Vec<(MediaTime, crate::model::MarkerKind)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Lanes {
    rows: usize,
    items: BTreeMap<Id, usize>,
}

impl Default for Lanes {
    /// Empty groups retain one labelled drop/authoring row in the native timeline.
    fn default() -> Self {
        Self {
            rows: 1,
            items: BTreeMap::new(),
        }
    }
}

impl TimelineLanes {
    /// Empty default publications compare without traversing item maps or relying on allocation identity.
    pub fn is_empty(&self) -> bool {
        let Self {
            cameras,
            effects,
            audio,
            titles,
            source_names,
            tracks: _,
            markers,
        } = self;
        cameras.items.is_empty()
            && effects.items.is_empty()
            && audio.items.is_empty()
            && titles.items.is_empty()
            && source_names.is_empty()
            && markers.is_empty()
    }

    /// Visible clip labels use one cached source lookup instead of a nested source scan per clip/frame.
    pub fn source_name(&self, id: Id) -> Option<&str> {
        self.source_names.get(&id).map(AsRef::as_ref)
    }

    /// Construct once per admitted project revision, never during per-frame timeline painting.
    pub fn new(project: &Project) -> Result<Self, StudioError> {
        project.validate()?;
        let audio = project
            .audio
            .iter()
            .map(|clip| clip.span().map(|span| (clip.id, span)))
            .collect::<Result<Vec<_>, _>>()?;
        let mut lanes = Self {
            tracks: Arc::from([]),
            markers: project
                .markers
                .iter()
                .map(|marker| (marker.at, marker.kind))
                .collect(),
            source_names: project
                .sources
                .iter()
                .map(|source| (source.id, Arc::from(source.name.as_str())))
                .collect(),
            cameras: Lanes::pack(
                project
                    .shots
                    .iter()
                    .map(|shot| (shot.id, shot.span))
                    .collect(),
            ),
            effects: Lanes::pack(
                project
                    .effects
                    .iter()
                    .map(|effect| (effect.id, effect.span))
                    .collect(),
            ),
            audio: Lanes::pack(audio),
            titles: Lanes::pack(
                project
                    .titles
                    .iter()
                    .map(|title| (title.id, title.span))
                    .collect(),
            ),
        };
        let visual_tracks = project
            .clips
            .iter()
            .map(|clip| clip.track + 1)
            .max()
            .unwrap_or(1);
        let mut tracks = (0..visual_tracks)
            .map(crate::tracks::Track::Video)
            .collect::<Vec<_>>();
        for (track, rows) in [
            (crate::tracks::Track::Camera, lanes.cameras.rows()),
            (crate::tracks::Track::Effects, lanes.effects.rows()),
            (crate::tracks::Track::Audio, lanes.audio.rows()),
            (crate::tracks::Track::Titles, lanes.titles.rows()),
        ] {
            tracks.extend(std::iter::repeat_n(track, rows));
        }
        lanes.tracks = tracks.into();
        lanes.markers.sort_by_key(|(at, _)| *at);
        Ok(lanes)
    }

    /// Queries only the visible ordered marker interval without allocating during timeline painting.
    pub fn visible_markers(
        &self,
        start: MediaTime,
        end: MediaTime,
    ) -> &[(MediaTime, crate::model::MarkerKind)] {
        let first = self.markers.partition_point(|(at, _)| *at < start);
        let last = self.markers.partition_point(|(at, _)| *at <= end);
        &self.markers[first..last.max(first)]
    }

    /// Physical rows derive from the admitted logical tracks and overlap packing exactly once.
    pub fn tracks(&self) -> &[crate::tracks::Track] {
        &self.tracks
    }

    /// Finds the first physical row for a stable logical owner.
    pub fn track_start(&self, track: crate::tracks::Track) -> usize {
        self.tracks
            .iter()
            .position(|candidate| *candidate == track)
            .unwrap_or(0)
    }
}

impl Lanes {
    /// The group keeps one visible row when empty and enough rows for every overlapping interval.
    pub const fn rows(&self) -> usize {
        self.rows
    }

    /// Item identity is stable across selection/transport changes; lane numbers are presentation only.
    pub fn row(&self, id: Id) -> Option<usize> {
        self.items.get(&id).copied()
    }

    /// Greedy interval partitioning uses O(n log n) work and reuses the lowest released row deterministically.
    fn pack(mut spans: Vec<(Id, Span)>) -> Self {
        spans.sort_unstable_by_key(|(id, span)| (span.start, *id));
        let mut busy: BinaryHeap<Reverse<(MediaTime, usize)>> = BinaryHeap::new();
        let mut free: BinaryHeap<Reverse<usize>> = BinaryHeap::new();
        let mut next = 0_usize;
        let mut items = BTreeMap::new();
        for (id, span) in spans {
            while busy
                .peek()
                .is_some_and(|Reverse((end, _))| *end <= span.start)
            {
                let Reverse((_, row)) = busy.pop().expect("peeked interval");
                free.push(Reverse(row));
            }
            let row = free.pop().map(|Reverse(row)| row).unwrap_or_else(|| {
                let row = next;
                next += 1;
                row
            });
            items.insert(id, row);
            busy.push(Reverse((span.end, row)));
        }
        Self {
            rows: next.max(1),
            items,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Confidence, Marker, MarkerKind};

    #[test]
    fn marker_windows_are_ordered_inclusive_and_do_not_depend_on_project_insertion_order() {
        let mut project = Project::new("Marker window");
        project.markers = (0..10_000)
            .rev()
            .map(|index| Marker {
                id: Id(index + 1),
                at: MediaTime(index * 1_000),
                kind: MarkerKind::Beat,
                label: String::new(),
                confidence: Confidence::Correlated,
                origin: None,
            })
            .collect();
        let lanes = TimelineLanes::new(&project).unwrap();
        let window = lanes.visible_markers(MediaTime(5_000_000), MediaTime(5_003_000));
        assert_eq!(
            window.iter().map(|(at, _)| at.0).collect::<Vec<_>>(),
            [5_000_000, 5_001_000, 5_002_000, 5_003_000]
        );
        assert!(
            lanes
                .visible_markers(MediaTime(20_000_000), MediaTime(21_000_000))
                .is_empty()
        );
        assert!(
            lanes
                .visible_markers(MediaTime(8_000_000), MediaTime(7_000_000))
                .is_empty()
        );
        assert_eq!(project.markers[0].at, MediaTime(9_999_000));
    }
}
