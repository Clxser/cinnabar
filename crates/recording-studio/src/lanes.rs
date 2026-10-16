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
        } = self;
        cameras.items.is_empty()
            && effects.items.is_empty()
            && audio.items.is_empty()
            && titles.items.is_empty()
            && source_names.is_empty()
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
        Ok(Self {
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
        })
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
