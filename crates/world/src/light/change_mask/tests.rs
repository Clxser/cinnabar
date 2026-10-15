use super::*;

#[test]
fn changes_include_only_halo_regions_containing_the_changed_sample() {
    let previous = SubChunkLight::dark(1);
    let edge = SUB_CHUNK_SIDE - 1;
    for point in [
        [8, 8, 8],
        [edge, 8, 8],
        [edge, edge, 8],
        [edge, edge, edge],
        [0, 0, 0],
    ] {
        let mut next = previous.clone().with_generation(2);
        next.set(
            LightChannel::Sky,
            point[0] as u8,
            point[1] as u8,
            point[2] as u8,
            7,
        )
        .unwrap();
        let changed = next.mesh_changes_since(Some(&previous));
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    let expected = point
                        .into_iter()
                        .zip([x, y, z])
                        .all(|(coordinate, offset)| match offset {
                            -1 => coordinate == 0,
                            1 => coordinate == edge,
                            _ => true,
                        });
                    assert_eq!(
                        changed.contains([x, y, z]),
                        expected,
                        "sample {point:?}, offset {x}, {y}, {z}"
                    );
                }
            }
        }
        assert!(!changed.contains([i32::MAX, 0, 0]));
    }
}

#[test]
fn separate_face_changes_do_not_invalidate_their_unmodified_shared_edge() {
    let previous = SubChunkLight::dark(1);
    let mut next = previous.clone();
    next.set(LightChannel::Block, 15, 8, 8, 7).unwrap();
    next.set(LightChannel::Sky, 8, 15, 8, 13).unwrap();
    let changed = next.mesh_changes_since(Some(&previous));
    assert!(changed.contains([1, 0, 0]));
    assert!(changed.contains([0, 1, 0]));
    assert!(!changed.contains([1, 1, 0]));
    assert!(!changed.contains([-1, 0, 0]));
}

#[test]
fn generations_do_not_change_light_and_uniform_changes_cover_every_neighbour() {
    let previous = SubChunkLight::uniform(3, 15, 1).unwrap();
    assert_eq!(
        previous
            .clone()
            .with_generation(2)
            .mesh_changes_since(Some(&previous)),
        LightChangeMask::EMPTY
    );
    assert_eq!(previous.mesh_changes_since(None), LightChangeMask::ALL);
    assert_eq!(
        SubChunkLight::uniform(4, 15, 2)
            .unwrap()
            .mesh_changes_since(Some(&previous)),
        LightChangeMask::ALL
    );
}
