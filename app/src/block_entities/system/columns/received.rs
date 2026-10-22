//! Received-world admission walks changes and drawable columns, not a warm world's empty scans.

use std::collections::{BTreeMap, BTreeSet};

use bevy::platform::collections::HashMap;
use world::{
    Chunk, ChunkKey, ChunkStore, ReceivedColumnChange, ReceivedColumnCursor, ReceivedColumnWork,
};

use super::super::{MAX_SUBMISSIONS, recording::MAX_CACHED_COLUMNS};
use super::{ColumnScan, MAX_COLUMN_RESCANS_PER_FRAME, RescanOrder};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Cost {
    bytes: usize,
    entities: usize,
}

impl Cost {
    /// Admission reads maintained NBT totals without retaining or parsing any records.
    fn of(chunk: &Chunk) -> Self {
        Self {
            bytes: chunk.block_entity_bytes(),
            entities: chunk.block_entities().len(),
        }
    }
}

#[derive(Default)]
pub(in crate::block_entities) struct ReceivedColumns {
    cursor: ReceivedColumnCursor,
    admitted: BTreeMap<ChunkKey, Cost>,
    dirty: BTreeSet<ChunkKey>,
    active: BTreeSet<ChunkKey>,
    changes: Vec<ChunkKey>,
    current: usize,
    bytes: usize,
    entities: usize,
    limited: Option<ChunkKey>,
}

impl ReceivedColumns {
    /// Applies membership/cost changes in sorted admission order and invalidates only changed scans.
    /// Full membership admission is needed only for a reset or sorted capacity reselection.
    pub(in crate::block_entities) fn synchronize(
        &mut self,
        store: &ChunkStore,
        dimension: i32,
        scans: &mut HashMap<ChunkKey, ColumnScan>,
    ) -> ReceivedColumnWork {
        self.changes.clear();
        let mut reset = false;
        let work =
            store.visit_received_column_changes(&mut self.cursor, dimension, |event| match event {
                ReceivedColumnChange::Reset => reset = true,
                ReceivedColumnChange::Column(key) if !reset => self.changes.push(key),
                ReceivedColumnChange::Column(_) => {}
            });
        let rebuild = reset || self.update_admission(store, scans);
        if rebuild {
            self.rebuild_admission(store, dimension, scans);
        }
        for key in self.changes.iter().copied() {
            if self.admitted.contains_key(&key)
                && store
                    .chunk(key)
                    .is_some_and(|chunk| scans.get(&key).is_none_or(|scan| !scan.is_current(chunk)))
                && self.dirty.insert(key)
            {
                self.current -= 1;
            }
        }
        work
    }

    /// Within-budget membership and NBT edits update their own records without a world walk.
    /// A saturated cache replays sorted admission when a changed cost can release or consume room.
    fn update_admission(
        &mut self,
        store: &ChunkStore,
        scans: &mut HashMap<ChunkKey, ColumnScan>,
    ) -> bool {
        for key in self.changes.iter().copied() {
            let before = self.admitted.get(&key).copied();
            let after = store.chunk(key).map(Cost::of);
            if before == after {
                continue;
            }
            if self.limited.is_some() {
                return true;
            }
            let bytes = self.bytes - before.map_or(0, |cost| cost.bytes)
                + after.map_or(0, |cost| cost.bytes);
            let entities = self.entities - before.map_or(0, |cost| cost.entities)
                + after.map_or(0, |cost| cost.entities);
            if after.is_some()
                && (self.admitted.len() + usize::from(before.is_none()) > MAX_CACHED_COLUMNS
                    || bytes > world::MAX_BLOCK_ENTITY_BYTES_PER_CHUNK
                    || entities > MAX_SUBMISSIONS * 4)
            {
                return true;
            }
            self.bytes = bytes;
            self.entities = entities;
            if let Some(cost) = after {
                self.admitted.insert(key, cost);
                if before.is_none() {
                    self.dirty.insert(key);
                }
            } else {
                self.admitted.remove(&key);
                scans.remove(&key);
                self.active.remove(&key);
                if !self.dirty.remove(&key) {
                    self.current -= 1;
                }
            }
        }
        false
    }

    /// Current weak-map identity is checked only at a source change or a reset, never per empty frame.
    fn rebuild_admission(
        &mut self,
        store: &ChunkStore,
        dimension: i32,
        scans: &mut HashMap<ChunkKey, ColumnScan>,
    ) {
        let (mut columns, mut bytes, mut entities) = (0, 0, 0);
        self.limited = None;
        // Existing records carry their allocation. The monotonic walk removes obsolete entries
        // after admission rather than rebuilding a second map of the retained world.
        let mut retained = BTreeSet::new();
        for key in store.received_columns_in_dimension(dimension) {
            let chunk = store
                .chunk(key)
                .expect("received index owns sparse membership");
            let cost = Cost::of(chunk);
            if columns >= MAX_CACHED_COLUMNS
                || bytes + cost.bytes > world::MAX_BLOCK_ENTITY_BYTES_PER_CHUNK
                || entities + cost.entities > MAX_SUBMISSIONS * 4
            {
                self.limited.get_or_insert(key);
                continue;
            }
            columns += 1;
            bytes += cost.bytes;
            entities += cost.entities;
            retained.insert(key);
            self.admitted.insert(key, cost);
            let scan = scans.get(&key);
            if scan.is_none_or(|scan| !scan.is_current(chunk)) {
                self.dirty.insert(key);
            } else {
                self.dirty.remove(&key);
            }
            if scan.is_some_and(|scan| !scan.portals.is_empty() || !scan.entities.is_empty()) {
                self.active.insert(key);
            } else {
                self.active.remove(&key);
            }
        }
        self.admitted.retain(|key, _| retained.contains(key));
        scans.retain(|key, _| retained.contains(key));
        self.dirty.retain(|key| retained.contains(key));
        self.active.retain(|key| retained.contains(key));
        self.current = self.admitted.len() - self.dirty.len();
        self.bytes = bytes;
        self.entities = entities;
    }

    /// Selects the same sorted round-robin order without scanning clean columns before the cursor.
    pub(in crate::block_entities) fn select(
        &self,
        order: &mut RescanOrder,
    ) -> [Option<ChunkKey>; MAX_COLUMN_RESCANS_PER_FRAME] {
        order.select_set(&self.dirty)
    }

    /// Publishes a completed scan, including an empty negative result, exactly once per revision.
    pub(in crate::block_entities) fn scanned(&mut self, key: ChunkKey, scan: &ColumnScan) {
        if self.dirty.remove(&key) {
            self.current += 1;
        }
        if scan.portals.is_empty() && scan.entities.is_empty() {
            self.active.remove(&key);
        } else {
            self.active.insert(key);
        }
    }

    /// A rejected portal scan is retried with the original bounded round-robin policy.
    pub(in crate::block_entities) fn defer(&mut self, key: ChunkKey) {
        if self.dirty.insert(key) {
            self.current -= 1;
        }
        self.active.remove(&key);
    }

    /// A full submission frame historically drops later scans; preserve that admission behavior.
    pub(in crate::block_entities) fn discard_after(
        &mut self,
        key: ChunkKey,
        scans: &mut HashMap<ChunkKey, ColumnScan>,
    ) {
        use std::ops::Bound::{Excluded, Unbounded};
        for (&key, _) in self.admitted.range((Excluded(key), Unbounded)) {
            scans.remove(&key);
            self.active.remove(&key);
            if self.dirty.insert(key) {
                self.current -= 1;
            }
        }
    }

    /// Only these columns require animation, canvas, lighting and submission work each frame.
    pub(in crate::block_entities) fn active(&self) -> impl Iterator<Item = ChunkKey> + '_ {
        self.active.iter().copied()
    }

    /// Exact admitted scans whose palettes and NBT match the last observed store revision.
    pub(in crate::block_entities) fn current(&self) -> u32 {
        self.current as u32
    }

    /// Whether any sparse column was denied by the original retained-content budgets.
    pub(in crate::block_entities) fn limited(&self) -> bool {
        self.limited.is_some()
    }

    /// Full submission frames only inspect admission through the column at which they stop.
    pub(in crate::block_entities) fn limited_through(&self, key: ChunkKey) -> bool {
        self.limited.is_some_and(|first| first <= key)
    }
}

#[cfg(test)]
mod tests;
