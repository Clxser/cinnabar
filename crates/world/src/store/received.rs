//! Sparse membership and bounded invalidation history for independent world consumers.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Weak},
};

use super::ChunkStore;
use crate::ChunkKey;

/// History is an acceleration cache; a lagging consumer receives a complete reset.
const CHANGE_HISTORY: usize = 4_096;

/// Independent consumer position, invalidated by a different store or dimension.
#[derive(Debug, Default)]
pub struct ReceivedColumnCursor {
    identity: Weak<()>,
    dimension: Option<i32>,
    sequence: u64,
}

/// A reset precedes the current sparse membership; a column event reads its latest state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceivedColumnChange {
    Reset,
    Column(ChunkKey),
}

/// Deterministic consumer work, excluding the reset notification.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReceivedColumnWork {
    pub reset: bool,
    pub visited: usize,
}

#[derive(Debug)]
pub(super) struct ReceivedColumns {
    columns: BTreeMap<i32, BTreeSet<ChunkKey>>,
    identity: Arc<()>,
    sequence: u64,
    history: VecDeque<(u64, ChunkKey)>,
}

impl Default for ReceivedColumns {
    fn default() -> Self {
        Self {
            columns: BTreeMap::new(),
            identity: Arc::new(()),
            sequence: 0,
            history: VecDeque::new(),
        }
    }
}

impl ReceivedColumns {
    /// Historical collision snapshots build this index only if a consumer requests it.
    fn from_keys(keys: impl Iterator<Item = ChunkKey>) -> Self {
        let mut index = Self::default();
        for key in keys {
            index.columns.entry(key.dimension).or_default().insert(key);
        }
        index
    }

    /// Changes publication never walks another column or copies an immutable index.
    fn publish(&mut self, key: ChunkKey, present: bool) {
        if present {
            self.columns.entry(key.dimension).or_default().insert(key);
        } else if let Some(columns) = self.columns.get_mut(&key.dimension) {
            columns.remove(&key);
            if columns.is_empty() {
                self.columns.remove(&key.dimension);
            }
        }
        self.sequence = match self.sequence.checked_add(1) {
            Some(next) => next,
            None => {
                self.identity = Arc::new(());
                self.history.clear();
                1
            }
        };
        if self.history.len() == CHANGE_HISTORY {
            self.history.pop_front();
        }
        self.history.push_back((self.sequence, key));
    }
}

impl ChunkStore {
    /// Exact received sparse count, including NBT-only columns but excluding authority-only air.
    pub fn received_column_count(&self, dimension: i32) -> usize {
        self.received_index()
            .columns
            .get(&dimension)
            .map_or(0, BTreeSet::len)
    }

    /// Sorted sparse membership in one dimension; no camera or replay admission policy is applied.
    pub fn received_columns_in_dimension(
        &self,
        dimension: i32,
    ) -> impl Iterator<Item = ChunkKey> + '_ {
        self.received_index()
            .columns
            .get(&dimension)
            .into_iter()
            .flat_map(|columns| columns.iter().copied())
    }

    /// Visits every change since this consumer's last call, then advances its cursor.
    /// A reset replaces prior membership; overflow never drops data from the store.
    /// Multiple writes to a column remain distinct notifications, with its current state read
    /// from `chunk`, matching observation after the enclosing atomic store commits.
    pub fn visit_received_column_changes(
        &self,
        cursor: &mut ReceivedColumnCursor,
        dimension: i32,
        mut visit: impl FnMut(ReceivedColumnChange),
    ) -> ReceivedColumnWork {
        let index = self.received_index();
        let reset = !Weak::ptr_eq(&cursor.identity, &Arc::downgrade(&index.identity))
            || cursor.dimension != Some(dimension)
            || cursor.sequence > index.sequence
            || index
                .history
                .front()
                .is_some_and(|(first, _)| cursor.sequence < first.saturating_sub(1));
        let mut work = ReceivedColumnWork { reset, visited: 0 };
        if reset {
            visit(ReceivedColumnChange::Reset);
            for key in self.received_columns_in_dimension(dimension) {
                visit(ReceivedColumnChange::Column(key));
                work.visited += 1;
            }
        } else {
            // Sequence differences directly address the retained ring, so a warm call does not
            // inspect history, and an incremental call reads only its new suffix.
            let count = usize::try_from(index.sequence - cursor.sequence)
                .expect("retained history bounds the sequence difference");
            for at in index.history.len() - count..index.history.len() {
                let (_, key) = index.history[at];
                if key.dimension == dimension {
                    visit(ReceivedColumnChange::Column(key));
                    work.visited += 1;
                }
            }
        }
        cursor.identity = Arc::downgrade(&index.identity);
        cursor.dimension = Some(dimension);
        cursor.sequence = index.sequence;
        work
    }

    /// Sparse writes publish only after their fallible validation and actual mutation complete.
    pub(super) fn publish_received_column(&mut self, key: ChunkKey) {
        let present = self.chunks.contains_key(&key);
        if let Some(index) = self.received.get_mut() {
            index.publish(key, present);
        } else {
            // Collision snapshots omit this unused index until their first query or mutation.
            self.received = ReceivedColumns::from_keys(self.chunks.keys().copied()).into();
        }
    }

    /// Live stores are eager; historical snapshots avoid copying an index prediction never reads.
    fn received_index(&self) -> &ReceivedColumns {
        self.received
            .get_or_init(|| ReceivedColumns::from_keys(self.chunks.keys().copied()))
    }
}

#[cfg(test)]
mod tests;
