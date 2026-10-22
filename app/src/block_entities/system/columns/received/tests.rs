use super::*;
use std::sync::Arc;
use world::{
    BlockEntityKey, BlockEntityNbt, BlockUpdate, DecodedBiomeColumn, RawBiomeIds, SubChunkKey,
};

use super::super::super::{BlockEntityRuntime, Template, portals};
use render_model::BlockEntityKind;

/// Biome-only sparse columns have no portals or NBT, but still count as received.
fn empty_world(count: i32) -> ChunkStore {
    let mut store = ChunkStore::new();
    for x in 0..count {
        store.commit_biome_column(
            ChunkKey::new(0, x, 0),
            DecodedBiomeColumn::decode(0, 1, &[1, 0], &RawBiomeIds { default_biome: 0 }),
        );
    }
    store
}

/// Valid positionless NBT exercises packet-56 admission without external carriers.
fn sign(text: &str) -> BlockEntityNbt {
    let mut bytes = vec![10, 0];
    for (name, value) in [("id", "Sign"), ("Text", text)] {
        bytes.extend_from_slice(&[8, name.len() as u8]);
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(value.len() as u8);
        bytes.extend_from_slice(value.as_bytes());
    }
    bytes.push(0);
    BlockEntityNbt::decode_prefix(&bytes).unwrap().0
}

/// Real native palette scans and NBT routing feed the same admission methods as frame preparation.
fn sample(
    cache: &mut ReceivedColumns,
    store: &ChunkStore,
    dimension: i32,
    scans: &mut HashMap<ChunkKey, ColumnScan>,
    order: &mut RescanOrder,
) -> ReceivedColumnWork {
    let work = cache.synchronize(store, dimension, scans);
    for key in cache.select(order).into_iter().flatten() {
        let chunk = store.chunk(key).unwrap();
        let portals = portals::column_cells_limited(key, chunk, MAX_SUBMISSIONS, |id| {
            (id == 99).then_some(BlockEntityKind::EndPortal)
        });
        let entities = super::super::routed_entities(chunk, scans.remove(&key), |_, _, _, _| {
            Some(Template::Static(BlockEntityKind::EndGateway))
        });
        let scan = ColumnScan::new(chunk, portals, entities);
        cache.scanned(key, &scan);
        scans.insert(key, scan);
    }
    work
}

#[test]
fn thousands_of_unchanged_empty_columns_have_no_selection_scan_churn_or_allocations() {
    let count = 5_000;
    let store = empty_world(count);
    let mut cache = ReceivedColumns::default();
    let mut scans = HashMap::default();
    let mut order = RescanOrder::default();
    for _ in 0..(count as usize).div_ceil(MAX_COLUMN_RESCANS_PER_FRAME) {
        sample(&mut cache, &store, 0, &mut scans, &mut order);
    }
    assert_eq!(cache.current(), count as u32);
    assert_eq!(scans.len(), count as usize);
    assert_eq!(cache.active().count(), 0);
    assert!(cache.dirty.is_empty());
    let key = ChunkKey::new(0, count / 2, 0);
    let address = &scans[&key] as *const ColumnScan;
    let capacity = scans.capacity();
    let (_, allocations) = test_alloc::measure(|| {
        for _ in 0..120 {
            assert_eq!(
                sample(&mut cache, &store, 0, &mut scans, &mut order),
                ReceivedColumnWork::default()
            );
            assert_eq!(cache.current(), count as u32);
            assert_eq!(cache.active().count(), 0);
            assert_eq!(&scans[&key] as *const ColumnScan, address);
            assert_eq!(scans.capacity(), capacity);
        }
    });
    assert_eq!(allocations.calls, 0);
}

#[test]
fn late_portal_and_sign_changes_are_budgeted_without_losing_empty_column_coverage() {
    let count = 3 * MAX_COLUMN_RESCANS_PER_FRAME;
    let mut store = empty_world(count as i32);
    let mut cache = ReceivedColumns::default();
    let mut scans = HashMap::default();
    let mut order = RescanOrder::default();
    for _ in 0..3 {
        sample(&mut cache, &store, 0, &mut scans, &mut order);
    }
    for x in 0..count as i32 {
        store
            .update_block(
                SubChunkKey::new(0, x, 4, 0),
                BlockUpdate::new(0, 0, 0, 0, 99),
                0,
            )
            .unwrap();
    }
    cache.synchronize(&store, 0, &mut scans);
    assert_eq!(cache.current(), 0);
    assert_eq!(
        cache.active().count(),
        0,
        "stale empty scans wait for bounded refresh"
    );
    for round in 1..=3 {
        sample(&mut cache, &store, 0, &mut scans, &mut order);
        assert_eq!(
            cache.current() as usize,
            round * MAX_COLUMN_RESCANS_PER_FRAME
        );
        assert_eq!(cache.active().count(), round * MAX_COLUMN_RESCANS_PER_FRAME);
    }
    let entity = BlockEntityKey::new(0, 0, 64, 0);
    store
        .commit_block_entity_update(entity, sign("first"))
        .unwrap();
    assert_eq!(
        sample(&mut cache, &store, 0, &mut scans, &mut order).visited,
        1
    );
    assert_eq!(scans[&entity.chunk()].entities.len(), 1);
    let old_nbt = Arc::clone(&scans[&entity.chunk()].entities[0].nbt);
    store
        .commit_block_entity_update(entity, sign("longer edited text"))
        .unwrap();
    assert_eq!(
        sample(&mut cache, &store, 0, &mut scans, &mut order).visited,
        1
    );
    assert!(!Arc::ptr_eq(
        &old_nbt,
        &scans[&entity.chunk()].entities[0].nbt
    ));
    assert_eq!(cache.current() as usize, count);
    assert!(!cache.limited());
    // Separate samples retain the visible portal-on / portal-off / portal-on transitions.
    let section = SubChunkKey::from_chunk(entity.chunk(), 4);
    for id in [1, 99, 1] {
        store
            .update_block(section, BlockUpdate::new(0, 0, 0, 0, id), 0)
            .unwrap();
        sample(&mut cache, &store, 0, &mut scans, &mut order);
        assert_eq!(scans[&entity.chunk()].portals.len(), usize::from(id == 99));
    }
}

#[test]
fn membership_changes_within_budget_update_only_the_changed_column_and_dimension_resets() {
    let mut store = empty_world(32);
    let mut cache = ReceivedColumns::default();
    let mut scans = HashMap::default();
    let mut order = RescanOrder::default();
    for _ in 0..4 {
        sample(&mut cache, &store, 0, &mut scans, &mut order);
    }
    let removed = ChunkKey::new(0, 10, 0);
    store.evict_chunk(removed);
    let work = sample(&mut cache, &store, 0, &mut scans, &mut order);
    assert_eq!(
        work,
        ReceivedColumnWork {
            reset: false,
            visited: 1
        }
    );
    assert_eq!(cache.current(), 31);
    assert!(!scans.contains_key(&removed));
    let new_key = BlockEntityKey::new(0, 16_000, 64, 0);
    store
        .commit_block_entity_update(new_key, sign("NBT only"))
        .unwrap();
    assert_eq!(
        sample(&mut cache, &store, 0, &mut scans, &mut order).visited,
        1
    );
    assert_eq!(cache.current(), 32);
    // An unload/reload between samples has equal costs but a different map identity.
    store.evict_chunk(new_key.chunk());
    store
        .commit_block_entity_update(new_key, sign("reloaded"))
        .unwrap();
    assert_eq!(
        sample(&mut cache, &store, 0, &mut scans, &mut order).visited,
        2
    );
    assert!(scans[&new_key.chunk()].is_current(store.chunk(new_key.chunk()).unwrap()));
    assert_eq!(cache.current(), 32);
    assert!(sample(&mut cache, &store, 1, &mut scans, &mut order).reset);
    assert_eq!(cache.current(), 0);
    assert!(scans.is_empty());
    assert!(sample(&mut cache, &store, 0, &mut scans, &mut order).reset);
    assert_eq!(cache.current() as usize, MAX_COLUMN_RESCANS_PER_FRAME);
    assert_eq!(cache.admitted.len(), 32);
    assert!(sample(&mut cache, &ChunkStore::new(), 0, &mut scans, &mut order).reset);
    assert!(cache.admitted.is_empty());
}

#[test]
fn sorted_dirty_index_matches_the_original_round_robin_and_retries_rejected_scans() {
    let count = 3 * MAX_COLUMN_RESCANS_PER_FRAME;
    let mut store = empty_world(count as i32);
    let mut cache = ReceivedColumns::default();
    let mut scans = HashMap::default();
    cache.synchronize(&store, 0, &mut scans);
    let mut previous = RescanOrder::default();
    let mut indexed = RescanOrder::default();
    for _ in 0..5 {
        assert_eq!(
            cache.select(&mut indexed),
            previous.select(cache.dirty.iter().copied())
        );
    }
    for _ in 0..3 {
        sample(&mut cache, &store, 0, &mut scans, &mut indexed);
    }
    let key = ChunkKey::new(0, 0, 0);
    cache.defer(key);
    scans.remove(&key);
    assert_eq!(cache.current() as usize, count - 1);
    assert_eq!(
        cache
            .select(&mut indexed)
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>(),
        [key]
    );
    sample(&mut cache, &store, 0, &mut scans, &mut indexed);
    assert_eq!(cache.current() as usize, count);
    cache.discard_after(key, &mut scans);
    assert_eq!(cache.current(), 1);
    assert_eq!(scans.len(), 1);
    store.evict_chunk(key);
    sample(&mut cache, &store, 0, &mut scans, &mut indexed);
    assert_eq!(cache.current() as usize, MAX_COLUMN_RESCANS_PER_FRAME);
}

#[test]
fn session_pack_or_dimension_height_change_invalidates_received_admission_without_store_events() {
    let store = empty_world(1);
    let mut runtime = BlockEntityRuntime::new();
    runtime.bind_session(Some((1, 0)));
    sample(
        &mut runtime.received_columns,
        &store,
        0,
        &mut runtime.columns,
        &mut runtime.rescans,
    );
    assert_eq!(runtime.received_columns.current(), 1);
    runtime.bind_session(Some((2, 0)));
    assert!(runtime.columns.is_empty());
    assert_eq!(runtime.received_columns.current(), 0);
    sample(
        &mut runtime.received_columns,
        &store,
        0,
        &mut runtime.columns,
        &mut runtime.rescans,
    );
    let assets = Arc::new(assets::RuntimeAssets::diagnostic());
    runtime.bind_beam_source(&assets, None);
    assert!(runtime.columns.is_empty());
    assert_eq!(runtime.received_columns.current(), 0);
    sample(
        &mut runtime.received_columns,
        &store,
        0,
        &mut runtime.columns,
        &mut runtime.rescans,
    );
    runtime.bind_beam_source(&assets, None);
    assert_eq!(
        runtime.received_columns.current(),
        1,
        "unchanged rules preserve ready scans"
    );
    runtime.bind_beam_source(
        &assets,
        Some(protocol::DimensionRange {
            base_sub_chunk_y: -4,
            sub_chunk_count: 24,
        }),
    );
    assert_eq!(runtime.received_columns.current(), 0);
    assert!(runtime.columns.is_empty());
    sample(
        &mut runtime.received_columns,
        &store,
        0,
        &mut runtime.columns,
        &mut runtime.rescans,
    );
    runtime.bind_beam_source(
        &Arc::new(assets::RuntimeAssets::diagnostic()),
        Some(protocol::DimensionRange {
            base_sub_chunk_y: -4,
            sub_chunk_count: 24,
        }),
    );
    assert_eq!(runtime.received_columns.current(), 0);
    assert!(runtime.columns.is_empty());
}

#[test]
fn saturated_admission_preserves_sorted_priority_counts_and_recovers_room_after_unload() {
    let mut store = empty_world(3);
    let nbt = sign("");
    // Fill the existing global entity budget using legitimate sparse NBT, without block arrays.
    for index in 0..MAX_SUBMISSIONS * 4 {
        let side = world::SUB_CHUNK_SIDE;
        let key = BlockEntityKey::new(
            0,
            (index % side) as i32,
            (index / (side * side)) as i32,
            ((index / side) % side) as i32,
        );
        store.commit_block_entity_update(key, nbt.clone()).unwrap();
    }
    let skipped = BlockEntityKey::new(0, world::SUB_CHUNK_SIDE as i32, 0, 0);
    store.commit_block_entity_update(skipped, nbt).unwrap();
    let mut cache = ReceivedColumns::default();
    let mut scans = HashMap::default();
    cache.synchronize(&store, 0, &mut scans);
    assert_eq!(store.received_column_count(0), 3);
    assert_eq!(
        cache.admitted.keys().copied().collect::<Vec<_>>(),
        [ChunkKey::new(0, 0, 0), ChunkKey::new(0, 2, 0)]
    );
    assert!(cache.limited());
    assert!(!cache.limited_through(ChunkKey::new(0, 0, 0)));
    assert!(cache.limited_through(skipped.chunk()));
    assert_eq!(cache.current(), 0);
    store.evict_chunk(ChunkKey::new(0, 0, 0));
    let mut order = RescanOrder::default();
    sample(&mut cache, &store, 0, &mut scans, &mut order);
    assert_eq!(store.received_column_count(0), 2);
    assert_eq!(cache.admitted.len(), 2);
    assert_eq!(cache.current(), 2);
    assert!(!cache.limited());
    assert!(cache.admitted.contains_key(&skipped.chunk()));
}
