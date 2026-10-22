//! A balanced interval index prunes inactive work, including long audio and camera clips.

use crate::{MediaTime, Project, Span, StudioError};

#[derive(Clone, Copy)]
pub(super) enum Slot {
    Clip(usize),
    Camera(usize),
    Effect(usize),
    Audio(usize),
    Title(usize),
    Transition(usize),
}

#[derive(Clone, Copy)]
struct Entry {
    span: Span,
    slot: Slot,
}

struct Node {
    entry: Entry,
    maximum_end: MediaTime,
    left: Option<usize>,
    right: Option<usize>,
}

pub(super) struct IntervalIndex {
    nodes: Vec<Node>,
    root: Option<usize>,
}

impl IntervalIndex {
    /// Sorting happens on edits, not on frames or individual timeline scrubs.
    pub(super) fn new(project: &Project) -> Result<Self, StudioError> {
        let mut entries = Vec::new();
        for (index, clip) in project.clips.iter().enumerate() {
            entries.push(Entry {
                span: clip.span()?,
                slot: Slot::Clip(index),
            });
        }
        for (index, shot) in project.shots.iter().enumerate() {
            entries.push(Entry {
                span: shot.span,
                slot: Slot::Camera(index),
            });
        }
        for (index, effect) in project.effects.iter().enumerate() {
            entries.push(Entry {
                span: effect.span,
                slot: Slot::Effect(index),
            });
        }
        for (index, audio) in project.audio.iter().enumerate() {
            entries.push(Entry {
                span: audio.span()?,
                slot: Slot::Audio(index),
            });
        }
        for (index, title) in project.titles.iter().enumerate() {
            entries.push(Entry {
                span: title.span,
                slot: Slot::Title(index),
            });
        }
        for (index, transition) in project.transitions.iter().enumerate() {
            let clip = project
                .clips
                .iter()
                .find(|clip| clip.id == transition.to)
                .ok_or(StudioError::Missing {
                    kind: "clip",
                    id: transition.to.0,
                })?;
            entries.push(Entry {
                span: transition.span_at(clip.at)?,
                slot: Slot::Transition(index),
            });
        }
        entries.sort_by_key(|entry| entry.span.start);
        let mut result = Self {
            nodes: Vec::with_capacity(entries.len()),
            root: None,
        };
        result.root = result.build(&entries);
        Ok(result)
    }

    /// Visits half-open active intervals without constructing a transient result collection.
    pub(super) fn visit(&self, time: MediaTime, mut visitor: impl FnMut(Slot)) {
        self.walk(self.root, time, &mut visitor);
    }

    /// Builds a height-balanced tree using stable vector offsets instead of heap nodes.
    fn build(&mut self, entries: &[Entry]) -> Option<usize> {
        if entries.is_empty() {
            return None;
        }
        let middle = entries.len() / 2;
        let left = self.build(&entries[..middle]);
        let right = self.build(&entries[middle + 1..]);
        let maximum_end = [
            Some(entries[middle].span.end),
            left.map(|i| self.nodes[i].maximum_end),
            right.map(|i| self.nodes[i].maximum_end),
        ]
        .into_iter()
        .flatten()
        .max()
        .expect("one interval exists");
        let index = self.nodes.len();
        self.nodes.push(Node {
            entry: entries[middle],
            maximum_end,
            left,
            right,
        });
        Some(index)
    }

    /// Whole subtrees ending before the sample, or beginning after it, are never traversed.
    fn walk(&self, node: Option<usize>, time: MediaTime, visitor: &mut impl FnMut(Slot)) {
        let Some(node) = node.map(|index| &self.nodes[index]) else {
            return;
        };
        if node.maximum_end <= time {
            return;
        }
        self.walk(node.left, time, visitor);
        if node.entry.span.start > time {
            return;
        }
        if node.entry.span.contains(time) {
            visitor(node.entry.slot);
        }
        self.walk(node.right, time, visitor);
    }
}
