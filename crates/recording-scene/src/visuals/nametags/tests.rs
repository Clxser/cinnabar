use super::*;

/// Tests supply exact atlas bytes through the same bounded callback used by object-store admission.
fn metadata() -> Metadata {
    Metadata {
        revision: 7,
        cells: vec![Cell {
            cell: [2, 3, 2, 2],
            pixels: AssetId([1; 32]),
        }],
    }
}

/// One textured world label retains authored color, depth policy, multiline lift and atlas UVs.
fn record() -> NametagRecord {
    let side = NAMETAG_ATLAS_SIDE as f32;
    NametagRecord {
        anchor: [4.0, 65.0, 8.0],
        text: 1,
        rect: [-2.0, -1.0, 2.0, 1.0],
        uv: [2.0 / side, 3.0 / side, 4.0 / side, 5.0 / side],
        color: [0.3, 0.6, 0.9, 0.5],
        line_lift: 0.25,
        depth_tested: 1,
        padding: [0; 2],
    }
}

/// This is the public binary contract used by the visual-frame channel's prepared tag payload.
fn bytes(records: &[NametagRecord]) -> Vec<u8> {
    let mut bytes = Vec::new();
    binary::packed(&mut bytes, records, |record| {
        bytemuck::cast::<NametagRecord, [u32; RECORD_WORDS]>(*record)
    })
    .unwrap();
    bytes
}

#[test]
fn exact_world_tag_artwork_and_quads_survive_replay_independently_of_hud() {
    let record = record();
    let pixels: Arc<[u8]> = Arc::from([127; 16]);
    let mut loader = Loader::default();
    let mut budget = MAX_ATLAS_BYTES;
    let frame = load(
        &mut loader,
        metadata(),
        &bytes(&[record]),
        |_, expected| {
            assert_eq!(expected, pixels.len());
            Ok(Arc::clone(&pixels))
        },
        &mut budget,
        &|| false,
    )
    .unwrap();
    assert_eq!(frame.records, vec![record]);
    assert_eq!(frame.atlas[0].cell, [2, 3, 2, 2]);
    assert_eq!(frame.atlas[0].rgba8, pixels);
    assert_eq!(frame.atlas_revision, 7);
    let mut budget = MAX_ATLAS_BYTES;
    let next = load(
        &mut loader,
        metadata(),
        &bytes(&[record]),
        |_, _| panic!("unchanged atlas must retain its admitted pixels"),
        &mut budget,
        &|| false,
    )
    .unwrap();
    assert!(Arc::ptr_eq(&frame.atlas, &next.atlas));
}

#[test]
fn absent_world_tags_and_an_admitted_empty_frame_are_distinct() {
    let mut loader = Loader::default();
    let mut budget = MAX_ATLAS_BYTES;
    let frame = load(
        &mut loader,
        Metadata {
            revision: 0,
            cells: Vec::new(),
        },
        &bytes(&[]),
        |_, _| panic!("empty tag frame has no pixels"),
        &mut budget,
        &|| false,
    )
    .unwrap();
    assert!(frame.records.is_empty());
    assert!(frame.atlas.is_empty());
    assert!(crate::visuals::FrozenVisuals::default().nametags.is_none());
    let recorded = crate::visuals::FrozenVisuals {
        nametags: Some(frame),
        ..Default::default()
    };
    assert!(recorded.nametags.is_some());
}

#[test]
fn malformed_geometry_overlap_and_texel_bounds_fail_before_pixel_loading() {
    assert!(validate_cells([[0, 0, 2, 2], [1, 1, 2, 2]].into_iter()).is_err());
    assert!(validate_cells([[0, 0, 2, 2], [2, 0, 2, 2]].into_iter()).is_ok());
    assert!(validate_cells([[NAMETAG_ATLAS_SIDE, 0, 1, 1]].into_iter()).is_err());
    let mut malformed = record();
    malformed.anchor[0] = f32::NAN;
    let mut budget = MAX_ATLAS_BYTES;
    assert!(
        load(
            &mut Loader::default(),
            metadata(),
            &bytes(&[malformed]),
            |_, _| panic!("malformed records must fail before pixel loading"),
            &mut budget,
            &|| false
        )
        .is_err()
    );
    let mut trailing = bytes(&[record()]);
    trailing.push(0);
    assert!(
        load(
            &mut Loader::default(),
            metadata(),
            &trailing,
            |_, _| panic!("trailing bytes must fail before pixel loading"),
            &mut budget,
            &|| false
        )
        .is_err()
    );
}

#[test]
fn older_visual_metadata_has_no_manufactured_nametag_channel() {
    let metadata: crate::visuals::Metadata = serde_json::from_value(serde_json::json!({
        "particle_base": null, "particle_side": 0, "patch_seq": 0, "patches": [], "block_atlas": null,
        "dynamic": null, "revision": 0, "dynamic_revision": 0, "portal_star_rect": [0.0, 0.0, 0.0, 0.0], "portal_time_seconds": 0.0,
        "hand_enabled": false, "hand": null
    })).unwrap();
    assert!(metadata.nametags.is_none());
}
