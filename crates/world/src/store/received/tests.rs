use super::*;
use crate::{
    BlockEntityKey, BlockEntityNbt, BlockUpdate, DecodedBiomeColumn, DecodedBlockEntities,
    DecodedLevelChunk, DecodedSubChunk, RawBiomeIds, RawBlockIds, SubChunk, SubChunkKey,
};

const IDS: RawBlockIds = RawBlockIds { air: 0 };
const SECTION: SubChunkKey = SubChunkKey::new(0, -2, 4, 3);

#[global_allocator]
static ALLOCATOR: test_alloc::CountingAllocator = test_alloc::CountingAllocator;

/// One valid network NBT entity with explicit coordinates and a variable id.
fn entity(key: BlockEntityKey, id: &str) -> BlockEntityNbt {
    let mut bytes = vec![10, 0, 8, 2, b'i', b'd', id.len() as u8];
    bytes.extend_from_slice(id.as_bytes());
    for (name, value) in [b'x', b'y', b'z'].into_iter().zip(key.position()) {
        bytes.extend_from_slice(&[3, 1, name]);
        let mut value = ((value as u32) << 1) ^ ((value >> 31) as u32);
        loop {
            let mut byte = (value & 127) as u8;
            value >>= 7;
            byte |= if value == 0 { 0 } else { 128 };
            bytes.push(byte);
            if value == 0 {
                break;
            }
        }
    }
    bytes.push(0);
    BlockEntityNbt::decode_prefix(&bytes).unwrap().0
}

/// Counts and ordered membership must always agree with the owning sparse map.
fn assert_index(store: &ChunkStore) {
    let expected: BTreeSet<_> = store.chunks.keys().copied().collect();
    let actual: BTreeSet<_> = store
        .received_index()
        .columns
        .values()
        .flatten()
        .copied()
        .collect();
    assert_eq!(actual, expected);
    for dimension in [-1, 0, 1, 2] {
        assert_eq!(
            store.received_column_count(dimension),
            expected
                .iter()
                .filter(|key| key.dimension == dimension)
                .count()
        );
        assert_eq!(
            store
                .received_columns_in_dimension(dimension)
                .collect::<Vec<_>>(),
            expected
                .iter()
                .filter(|key| key.dimension == dimension)
                .copied()
                .collect::<Vec<_>>()
        );
    }
}

/// A consumer sees every committed dirty column, or a full reset with exact membership.
fn drain(store: &ChunkStore, cursor: &mut ReceivedColumnCursor) -> Vec<ReceivedColumnChange> {
    let mut changes = Vec::new();
    store.visit_received_column_changes(cursor, SECTION.dimension, |event| changes.push(event));
    assert_index(store);
    changes
}

#[test]
fn authority_only_air_never_becomes_received_and_nbt_only_columns_do() {
    let mut store = ChunkStore::new();
    let mut cursor = ReceivedColumnCursor::default();
    assert_eq!(drain(&store, &mut cursor), [ReceivedColumnChange::Reset]);
    store.mark_chunk_loaded(SECTION.chunk()).unwrap();
    store
        .mark_sub_chunk_loaded(SubChunkKey::new(1, 0, 0, 0))
        .unwrap();
    assert!(drain(&store, &mut cursor).is_empty());
    let key = BlockEntityKey::new(0, -31, 65, 49);
    let nbt = entity(key, "Sign");
    store.commit_block_entity_update(key, nbt.clone()).unwrap();
    assert_eq!(
        drain(&store, &mut cursor),
        [ReceivedColumnChange::Column(key.chunk())]
    );
    store.commit_block_entity_update(key, nbt).unwrap();
    assert!(
        drain(&store, &mut cursor).is_empty(),
        "identical NBT retains its identity"
    );
    store
        .commit_block_entity_update(key, entity(key, "Chest"))
        .unwrap();
    assert_eq!(
        drain(&store, &mut cursor),
        [ReceivedColumnChange::Column(key.chunk())]
    );
    store
        .commit_chunk_block_entities(
            key.chunk(),
            DecodedBlockEntities::decode_sub_chunk_tail(SECTION, &[]),
        )
        .unwrap();
    assert_eq!(
        drain(&store, &mut cursor),
        [ReceivedColumnChange::Column(key.chunk())]
    );
    assert_eq!(store.received_column_count(0), 0);
}

#[test]
fn palettes_prepared_mutations_air_eviction_and_reload_publish_every_scope() {
    let mut store = ChunkStore::new();
    let mut cursor = ReceivedColumnCursor::default();
    drain(&store, &mut cursor);
    for id in [1, 2] {
        store
            .update_block(SECTION, BlockUpdate::new(0, 0, 0, 0, id), 0)
            .unwrap();
        assert_eq!(
            drain(&store, &mut cursor),
            [ReceivedColumnChange::Column(SECTION.chunk())]
        );
    }
    let unchanged = ChunkStore::prepare_sub_chunk_blocks(
        SECTION,
        store.sub_chunk_ref(SECTION),
        &[BlockUpdate::new(0, 0, 0, 0, 2)],
        0,
    )
    .unwrap();
    store
        .commit_prepared_block_updates(vec![unchanged])
        .unwrap();
    assert!(drain(&store, &mut cursor).is_empty());
    store.apply_request_mode_air(SECTION).unwrap();
    assert_eq!(
        drain(&store, &mut cursor),
        [ReceivedColumnChange::Column(SECTION.chunk())]
    );
    store
        .apply_sub_chunk(SECTION, &[9, 1, 4, 1, 2], &IDS)
        .unwrap();
    assert_eq!(
        drain(&store, &mut cursor),
        [ReceivedColumnChange::Column(SECTION.chunk())]
    );
    store
        .commit_sub_chunk(SECTION, SubChunk::decode(&[9, 0, 4], &IDS))
        .unwrap();
    assert_eq!(
        drain(&store, &mut cursor),
        [ReceivedColumnChange::Column(SECTION.chunk())]
    );
    let prepared =
        ChunkStore::prepare_sub_chunk_blocks(SECTION, None, &[BlockUpdate::new(0, 0, 0, 0, 3)], 0)
            .unwrap();
    store.commit_prepared_block_updates(vec![prepared]).unwrap();
    assert_eq!(
        drain(&store, &mut cursor),
        [ReceivedColumnChange::Column(SECTION.chunk())]
    );
    let erased = ChunkStore::prepare_sub_chunk_blocks(
        SECTION,
        store.sub_chunk_ref(SECTION),
        &[BlockUpdate::new(0, 0, 0, 0, 0)],
        0,
    )
    .unwrap();
    store.commit_prepared_block_updates(vec![erased]).unwrap();
    assert_eq!(
        drain(&store, &mut cursor),
        [ReceivedColumnChange::Column(SECTION.chunk())]
    );
    store
        .apply_sub_chunk(SECTION, &[9, 1, 4, 1, 2], &IDS)
        .unwrap();
    drain(&store, &mut cursor);
    store.detach_chunks(&BTreeSet::from([SECTION.chunk()]));
    assert_eq!(
        drain(&store, &mut cursor),
        [ReceivedColumnChange::Column(SECTION.chunk())]
    );
    store
        .apply_sub_chunk(SECTION, &[9, 1, 4, 1, 2], &IDS)
        .unwrap();
    assert_eq!(
        drain(&store, &mut cursor),
        [ReceivedColumnChange::Column(SECTION.chunk())]
    );
    store.evict_chunk(SECTION.chunk());
    assert_eq!(
        drain(&store, &mut cursor),
        [ReceivedColumnChange::Column(SECTION.chunk())]
    );
}

#[test]
fn decoded_entity_replacements_inline_columns_and_biomes_keep_membership_exact() {
    let mut store = ChunkStore::new();
    let key = BlockEntityKey::new(0, -31, 65, 49);
    let mut cursor = ReceivedColumnCursor::default();
    drain(&store, &mut cursor);
    let nbt = entity(key, "Sign");
    let payload = [&[9, 1, 4, 1, 2][..], nbt.bytes()].concat();
    store
        .commit_decoded_sub_chunk(SECTION, DecodedSubChunk::decode(SECTION, &payload, &IDS))
        .unwrap();
    assert!(drain(&store, &mut cursor).contains(&ReceivedColumnChange::Column(SECTION.chunk())));
    store
        .commit_decoded_sub_chunk(
            SECTION,
            DecodedSubChunk::decode(SECTION, &[9, 1, 4, 1, 2], &IDS),
        )
        .unwrap();
    assert_eq!(
        drain(&store, &mut cursor),
        [ReceivedColumnChange::Column(SECTION.chunk())]
    );
    store
        .commit_chunk_block_entities(
            SECTION.chunk(),
            DecodedBlockEntities::decode_sub_chunk_tail(SECTION, nbt.bytes()),
        )
        .unwrap();
    assert_eq!(
        drain(&store, &mut cursor),
        [ReceivedColumnChange::Column(SECTION.chunk())]
    );
    store.apply_all_air(SECTION).unwrap();
    assert!(drain(&store, &mut cursor).contains(&ReceivedColumnChange::Column(SECTION.chunk())));
    assert!(store.chunk(SECTION.chunk()).is_none());
    store
        .apply_level_chunk(SECTION.chunk(), 4, 1, &[9, 1, 4, 1, 2], &IDS)
        .unwrap();
    assert_eq!(
        drain(&store, &mut cursor),
        [ReceivedColumnChange::Column(SECTION.chunk())]
    );
    store
        .commit_level_chunk(SECTION.chunk(), DecodedLevelChunk::decode(4, 0, &[], &IDS))
        .unwrap();
    assert_eq!(
        drain(&store, &mut cursor),
        [ReceivedColumnChange::Column(SECTION.chunk())]
    );
    store.commit_biome_column(
        SECTION.chunk(),
        DecodedBiomeColumn::decode(4, 1, &[1, 2], &RawBiomeIds { default_biome: 0 }),
    );
    assert_eq!(
        drain(&store, &mut cursor),
        [ReceivedColumnChange::Column(SECTION.chunk())]
    );
    store.apply_request_mode_air(SECTION).unwrap();
    assert!(
        drain(&store, &mut cursor).is_empty(),
        "a missing block section has no sparse mutation"
    );
}

#[test]
fn consumers_are_independent_store_dimension_and_snapshot_bound() {
    let mut store = ChunkStore::new();
    let mut first = ReceivedColumnCursor::default();
    let mut second = ReceivedColumnCursor::default();
    drain(&store, &mut first);
    drain(&store, &mut second);
    store
        .update_block(SECTION, BlockUpdate::new(0, 0, 0, 0, 1), 0)
        .unwrap();
    assert_eq!(drain(&store, &mut first), drain(&store, &mut second));
    assert!(drain(&store, &mut first).is_empty());
    let mut events = Vec::new();
    store.visit_received_column_changes(&mut first, 1, |event| events.push(event));
    assert_eq!(events, [ReceivedColumnChange::Reset]);
    assert_eq!(
        drain(&store, &mut first),
        [
            ReceivedColumnChange::Reset,
            ReceivedColumnChange::Column(SECTION.chunk())
        ]
    );
    let snapshot = store.collision_snapshot();
    assert!(
        snapshot.received.get().is_none(),
        "prediction does not copy an unused membership index"
    );
    assert_eq!(
        drain(&snapshot, &mut first),
        [
            ReceivedColumnChange::Reset,
            ReceivedColumnChange::Column(SECTION.chunk())
        ]
    );
    assert_eq!(
        drain(&ChunkStore::new(), &mut first),
        [ReceivedColumnChange::Reset]
    );
}

#[test]
fn overflow_rebuilds_current_membership_and_warm_queries_have_no_world_work_or_allocations() {
    let mut store = ChunkStore::new();
    let mut cursor = ReceivedColumnCursor::default();
    drain(&store, &mut cursor);
    for x in 0..CHANGE_HISTORY + 1 {
        let key = BlockEntityKey::new(0, x as i32 * 16, 64, 0);
        store
            .commit_block_entity_update(key, entity(key, "Sign"))
            .unwrap();
    }
    assert_eq!(store.received_index().history.len(), CHANGE_HISTORY);
    let changes = drain(&store, &mut cursor);
    assert_eq!(changes[0], ReceivedColumnChange::Reset);
    assert_eq!(
        changes.len(),
        CHANGE_HISTORY + 2,
        "history bounds never truncate sparse membership"
    );
    let (_, allocations) = test_alloc::measure(|| {
        for _ in 0..120 {
            let work = store.visit_received_column_changes(&mut cursor, 0, |_| {
                panic!("warm sparse store has no new events")
            });
            assert_eq!(work, ReceivedColumnWork::default());
            assert_eq!(store.received_column_count(0), CHANGE_HISTORY + 1);
        }
    });
    assert_eq!(allocations.calls, 0);
    // Exhaustion rotates identity rather than wrapping a cursor into a false cache hit.
    store.received.get_mut().unwrap().sequence = u64::MAX;
    store
        .commit_block_entity_update(
            BlockEntityKey::new(0, 0, 64, 0),
            entity(BlockEntityKey::new(0, 0, 64, 0), "Chest"),
        )
        .unwrap();
    assert_eq!(drain(&store, &mut cursor)[0], ReceivedColumnChange::Reset);
}

#[test]
fn rejected_mutations_publish_no_changes_or_membership() {
    let mut store = ChunkStore::new();
    let mut cursor = ReceivedColumnCursor::default();
    drain(&store, &mut cursor);
    assert!(
        store
            .update_block(SECTION, BlockUpdate::new(16, 0, 0, 0, 1), 0)
            .is_err()
    );
    let actual = BlockEntityKey::new(0, -31, 65, 49);
    let wrong = BlockEntityKey::new(0, 1, 65, 49);
    assert!(
        store
            .commit_block_entity_update(wrong, entity(actual, "Sign"))
            .is_err()
    );
    assert!(drain(&store, &mut cursor).is_empty());
    assert_eq!(store.received_column_count(0), 0);
}
