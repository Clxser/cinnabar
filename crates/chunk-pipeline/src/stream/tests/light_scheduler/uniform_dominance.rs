use super::*;

#[test]
fn uniform_dominance_skips_fallback_face_visits() {
    let source = SubChunkKey::new(1, 0, 0, 0);
    let mut stream = lit_stream(1);
    for offset in LIGHT_NEIGHBOUR_OFFSETS {
        let destination = offset_sub_chunk_key(source, offset).unwrap();
        for (from, to) in [
            ((0, 0, false), (0, 0, false)),
            ((15, 0, false), (14, 0, false)),
            ((0, 15, false), (0, 14, false)),
            ((0, 15, true), (0, 15, true)),
        ] {
            install_current_light(&mut stream, source, from.0, from.1, from.2);
            install_current_light(&mut stream, destination, to.0, to.1, to.2);

            let (dominates, visits) = lighting::with_dominance_face_visits(|| {
                stream.current_known_target_dominates_source_face(source, destination, [true; 6])
            });

            assert!(dominates, "offset={offset:?}, from={from:?}, to={to:?}");
            assert_eq!(visits, 0, "uniform dominance must not visit fallback cells");
        }
    }
}

#[test]
fn packed_channels_and_direct_masks_keep_exact_face_fallback() {
    let source = SubChunkKey::new(1, 0, 0, 0);
    let destination = SubChunkKey::new(1, 1, 0, 0);
    let mut stream = lit_stream(1);
    for input in 0..6 {
        install_current_light(&mut stream, source, 8, 8, false);
        install_current_light(&mut stream, destination, 8, 8, false);
        let key = if input < 2 || input == 4 {
            source
        } else {
            destination
        };
        if input < 4 {
            let mut light = stream.lighting.store.light(key).unwrap().as_ref().clone();
            let channel = if input % 2 == 0 {
                LightChannel::Block
            } else {
                LightChannel::Sky
            };
            light.set(channel, 8, 8, 8, 7).unwrap();
            stream.lighting.store.insert_known_air(key, light);
        } else {
            let mut words = Box::new([0_u64; 64]);
            let index = light_local_index(8, 8, 8);
            words[index / 64] |= 1 << (index % 64);
            stream.lighting.direct_sky.get_mut(&key).unwrap().mask =
                Arc::new(DirectSkyMask::Packed(words));
        }

        let (dominates, visits) = lighting::with_dominance_face_visits(|| {
            stream.current_known_target_dominates_source_face(source, destination, [true; 6])
        });

        assert!(
            dominates,
            "the changed interior cell does not touch the face"
        );
        assert_eq!(
            visits,
            world::SUB_CHUNK_SIDE.pow(2),
            "packed input {input} needs exact fallback"
        );
    }
}
