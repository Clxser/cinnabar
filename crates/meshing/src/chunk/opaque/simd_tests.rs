use super::*;

#[test]
fn contiguous_axis_masks_keep_all_six_face_coordinates() {
    let mut geometry = AxisColumns::empty();
    let mut blockers = AxisColumns::empty();
    for x in 0..SIDE {
        for y in 0..SIDE {
            for z in 0..SIDE {
                if (x * 13 + y * 7 + z * 3) % 5 < 3 {
                    geometry.set(x, y, z);
                }
                if (x * 3 + y * 11 + z * 17) % 7 < 4 {
                    blockers.set(x, y, z);
                }
            }
        }
    }
    for face in Face::ALL {
        let mut exposed = [[0; SIDE]; SIDE];
        simd_kernels::exposed_columns(
            geometry.columns(face).as_flattened(),
            blockers.columns(face).as_flattened(),
            face.is_negative(),
            FULL_COLUMN,
            exposed.as_flattened_mut(),
        );
        for (v, row) in exposed.iter().enumerate() {
            for (u, &column) in row.iter().enumerate() {
                for slice in 0..SIDE {
                    let [x, y, z] = block_coordinate(face, slice, u, v);
                    let neighbour = if face.is_negative() {
                        slice.checked_sub(1)
                    } else {
                        (slice + 1 < SIDE).then_some(slice + 1)
                    };
                    let blocked = neighbour.is_some_and(|slice| {
                        let [x, y, z] = block_coordinate(face, slice, u, v);
                        blockers.contains(x, y, z)
                    });
                    assert_eq!(
                        column & (1 << slice) != 0,
                        geometry.contains(x, y, z) && !blocked,
                        "face {face:?}, coordinate {x}, {y}, {z}",
                    );
                }
            }
        }
    }
}
