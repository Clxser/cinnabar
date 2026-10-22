use super::*;

/// Two native batches exercise a filtered HUD group and its screen-space composite.
fn fixture() -> (Metadata, Vec<u8>) {
    let page = UiTexturePage::owned([1, 1], Arc::from([255; 4])).unwrap();
    let input = UiRenderInput {
        revision: 19,
        viewport_size: [64, 64],
        safe_area: [0; 4],
        vertices: vec![
            UiRenderVertex {
                position: [0.0; 2],
                clip_z: 0.0,
                clip_w: 1.0,
                uv: [0.0; 2],
                color: [255; 4],
                style_flags: 0,
                alpha_cutoff: -1.0,
                model_light: 1.0,
                overlay_color: [0.0; 4],
            };
            3
        ]
        .into(),
        indices: vec![0, 1, 2, 0, 1, 2].into(),
        batches: vec![
            UiRenderBatch::new(
                0,
                UiScissor::new(0, 0, 64, 64),
                0,
                3,
                render_model::UI_BLEND_ALPHA,
            )
            .with_filter_group(Some(7)),
            UiRenderBatch::new(
                0,
                UiScissor::new(0, 0, 64, 64),
                3,
                3,
                render_model::UI_BLEND_FILTER,
            ),
        ]
        .into(),
        textures: Arc::new(UiTextureCatalog::new(vec![page], 1).unwrap()),
    };
    input.validate().unwrap();
    let metadata = Metadata {
        revision: input.revision,
        viewport: input.viewport_size,
        safe_area: input.safe_area,
        time_seconds: 1.25,
        glint_strength: 0.0,
        pages: vec![Page {
            dimensions: [1, 1],
            coverage: false,
            pixels: AssetId([1; 32]),
        }],
        glint: None,
        filter_groups: input
            .batches
            .iter()
            .map(|batch| batch.filter_group)
            .collect(),
    };
    let mut bytes = Vec::new();
    write_draws(&mut bytes, &input, &input.batches).unwrap();
    (metadata, bytes)
}

#[test]
fn recorded_hud_preserves_filter_ownership_without_changing_legacy_draw_framing() {
    let mut remaining = usize::MAX;
    let (metadata, bytes) = fixture();
    let metadata = serde_json::from_slice(&serde_json::to_vec(&metadata).unwrap()).unwrap();
    let restored = load(
        &mut Loader::default(),
        metadata,
        &bytes,
        |_, expected| {
            assert_eq!(expected, 4);
            Ok(Arc::from([255; 4]))
        },
        &mut remaining,
        &|| false,
    )
    .unwrap();
    assert_eq!(restored.input.batches[0].filter_group, Some(7));
    assert_eq!(restored.input.batches[1].filter_group, None);
    assert_eq!(
        restored.input.batches[1].blend_mode,
        render_model::UI_BLEND_FILTER
    );
    assert_eq!(restored.input.vertices.len(), 3);
    assert_eq!(&*restored.input.indices, &[0, 1, 2, 0, 1, 2]);
}

#[test]
fn legacy_hud_defaults_to_unfiltered_batches_and_mismatched_groups_fail_before_artwork() {
    let mut remaining = usize::MAX;
    let (metadata, bytes) = fixture();
    let mut encoded = serde_json::to_value(&metadata).unwrap();
    encoded.as_object_mut().unwrap().remove("filter_groups");
    let restored = load(
        &mut Loader::default(),
        serde_json::from_value(encoded).unwrap(),
        &bytes,
        |_, _| Ok(Arc::from([255; 4])),
        &mut remaining,
        &|| false,
    )
    .unwrap();
    assert!(
        restored
            .input
            .batches
            .iter()
            .all(|batch| batch.filter_group.is_none())
    );

    let (mut metadata, _) = fixture();
    metadata.filter_groups.truncate(1);
    let rejected = load(
        &mut Loader::default(),
        metadata,
        &bytes,
        |_, _| panic!("malformed draw ownership cannot load pixels"),
        &mut remaining,
        &|| false,
    );
    assert!(matches!(rejected, Err(ArchiveError::Invalid(_))));
}
