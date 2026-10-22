use super::*;

/// Entity-only columns exercise received coverage without requiring game carriers or dense blocks.
fn received_store() -> ChunkStore {
    let mut store = ChunkStore::new();
    for key in [
        BlockEntityKey::new(0, -16, 64, 0),
        BlockEntityKey::new(0, 0, 64, 16),
        BlockEntityKey::new(0, 32_000, 64, -32_000),
        BlockEntityKey::new(1, 0, 64, 0),
    ] {
        let nbt = BlockEntityNbt::decode_prefix(&[10, 0, 0]).unwrap().0;
        store.commit_block_entity_update(key, nbt).unwrap();
    }
    store
}

#[test]
fn received_keys_follow_unload_and_dimension_without_camera_clipping() {
    let mut store = received_store();
    let mut keys = Vec::new();
    collect_frame_keys(&mut keys, &store, 0, [0.0; 2], true);
    assert_eq!(
        keys,
        [
            ChunkKey::new(0, -1, 0),
            ChunkKey::new(0, 0, 1),
            ChunkKey::new(0, 2_000, -2_000)
        ]
    );
    let selected = keys.clone();
    collect_frame_keys(&mut keys, &store, 0, [1_000_000.0; 2], true);
    assert_eq!(
        keys, selected,
        "source camera position does not trim recorded coverage"
    );
    store.evict_chunk(ChunkKey::new(0, 2_000, -2_000));
    collect_frame_keys(&mut keys, &store, 0, [0.0; 2], true);
    assert_eq!(keys, [ChunkKey::new(0, -1, 0), ChunkKey::new(0, 0, 1)]);
    collect_frame_keys(&mut keys, &store, 1, [0.0; 2], true);
    assert_eq!(keys, [ChunkKey::new(1, 0, 0)]);
    collect_frame_keys(&mut keys, &store, 2, [0.0; 2], true);
    assert!(keys.is_empty());
}

#[test]
fn warm_received_selection_reuses_storage_without_per_sample_allocations() {
    const SAMPLES: usize = 120;
    let store = received_store();
    let mut keys = Vec::new();
    collect_frame_keys(&mut keys, &store, 0, [0.0; 2], true);
    let previous = keys.clone();
    let pointer = keys.as_ptr();
    let (_, allocations) = test_alloc::measure(|| {
        for _ in 0..SAMPLES {
            collect_frame_keys(&mut keys, &store, 0, [0.0; 2], true);
            assert_eq!(keys, previous);
            assert_eq!(keys.as_ptr(), pointer);
            let _ = std::hint::black_box(keys.as_slice());
        }
    });
    assert_eq!(allocations.calls, 0);
    let (_, previous_allocations) = test_alloc::measure(|| {
        for _ in 0..SAMPLES {
            let mut keys: Vec<_> = store
                .received_columns()
                .filter(|key| key.dimension == 0)
                .collect();
            keys.sort_unstable();
            drop(std::hint::black_box(keys));
        }
    });
    assert!(previous_allocations.calls >= SAMPLES);
}

#[test]
fn nearby_selection_keeps_the_previous_radius_and_refreshes_camera_coordinates() {
    let store = received_store();
    let mut keys = Vec::new();
    let radius = super::super::SCAN_RADIUS_BLOCKS;
    let side = SUB_CHUNK_SIDE as f32;
    let extent = (radius / side) as i32;
    let expected: Vec<_> = (-extent..=extent)
        .flat_map(|x| (-extent..=extent).map(move |z| ChunkKey::new(0, x, z)))
        .collect();
    collect_frame_keys(&mut keys, &store, 0, [0.0; 2], false);
    assert_eq!(keys, expected);
    let pointer = keys.as_ptr();
    collect_frame_keys(&mut keys, &store, 1, [side, -side], false);
    assert_eq!(keys.as_ptr(), pointer);
    assert_eq!(
        keys,
        expected
            .into_iter()
            .map(|key| ChunkKey::new(1, key.x + 1, key.z - 1))
            .collect::<Vec<_>>()
    );
}
