use super::*;

#[test]
fn nearby_selection_keeps_the_previous_radius_and_refreshes_camera_coordinates() {
    let mut keys = Vec::new();
    let radius = super::super::SCAN_RADIUS_BLOCKS;
    let side = SUB_CHUNK_SIDE as f32;
    let extent = (radius / side) as i32;
    let expected: Vec<_> = (-extent..=extent)
        .flat_map(|x| (-extent..=extent).map(move |z| ChunkKey::new(0, x, z)))
        .collect();
    collect_frame_keys(&mut keys, 0, [0.0; 2]);
    assert_eq!(keys, expected);
    let pointer = keys.as_ptr();
    collect_frame_keys(&mut keys, 1, [side, -side]);
    assert_eq!(keys.as_ptr(), pointer);
    assert_eq!(
        keys,
        expected
            .into_iter()
            .map(|key| ChunkKey::new(1, key.x + 1, key.z - 1))
            .collect::<Vec<_>>()
    );
}
