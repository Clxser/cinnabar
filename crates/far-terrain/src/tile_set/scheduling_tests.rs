use super::*;

#[test]
fn settled_tiles_leave_the_build_queue_and_changes_reenter_it() {
    let mut tiles = FarTiles::default();
    let mut store = SummaryStore::new(0);
    let now = Instant::now();
    tiles.set_view(TileView {
        centre: [8.0, 8.0],
        near_blocks: 0.0,
        skip_blocks: 0.0,
        far_blocks: 64.0,
    });
    for _ in 0..tiles.selected.len() {
        tiles.update(&mut store, now);
    }
    assert!(tiles.pending.is_empty());
    assert!(
        tiles
            .selected
            .values()
            .all(|tile| tile.mesh.is_some() && !tile.dirty)
    );
    let revision = tiles.revision();
    assert!(!tiles.update(&mut store, now));
    assert_eq!(tiles.revision(), revision);
    let key = *tiles.selected.keys().next().unwrap();
    let [x, z] = key
        .origin()
        .map(|block| block.div_euclid(COLUMN_SIDE as i32));
    tiles.mark_changed(&[ColumnPos::new(x, z)]);
    assert!(tiles.pending.contains(&key));
    tiles.clear();
    assert!(tiles.pending.is_empty());
}

#[test]
fn equivalent_tile_selections_keep_the_scene_revision() {
    let mut tiles = FarTiles::default();
    let view = TileView {
        centre: [8.0, 8.0],
        near_blocks: 0.0,
        skip_blocks: 0.0,
        far_blocks: 0.0,
    };
    tiles.set_view(view);
    let revision = tiles.revision();
    let next = TileView {
        centre: [8.001, 8.001],
        ..view
    };
    assert_eq!(select_tiles(view), select_tiles(next));
    tiles.set_view(next);
    assert_eq!(tiles.revision(), revision);
}
