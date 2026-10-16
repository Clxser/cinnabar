use super::*;

#[test]
fn identical_pixels_keep_distinct_state_source_identities() {
    let root = tempfile::tempdir().unwrap();
    for (name, text) in [
        ("blocks.json", "{}"),
        ("textures/flipbook_textures.json", "[]"),
        (
            "textures/terrain_texture.json",
            r#"{"texture_data":{"fixture":{"textures":["textures/same","textures/same"]}}}"#,
        ),
    ] {
        let path = root.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let pack = read_pack(root.path()).unwrap();
    let image = DecodedImage {
        source_path: "textures/same".into(),
        width: assets::TILE_SIZE,
        height: assets::TILE_SIZE,
        rgba8: vec![255; (assets::TILE_SIZE * assets::TILE_SIZE * 4) as usize].into(),
    };
    let plan = compile_animation_plan(
        &pack,
        &[image],
        AnimationLimits {
            max_layers_per_page: crate::animation::MAX_LAYERS_PER_PAGE,
            max_pages: 1,
        },
    )
    .unwrap();
    let descriptors = (0..2)
        .map(|state_variant| {
            (
                Descriptor {
                    path: "textures/same".into(),
                    texture_key: "fixture".into(),
                    flags: 0,
                    state_variant,
                    flipbook_selector: None,
                },
                "fixture".into(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let (materials, ids) = compile_materials(&descriptors, &plan, &BTreeSet::new()).unwrap();
    let ids_by_state = ids
        .iter()
        .map(|(descriptor, &id)| (descriptor.state_variant, id))
        .collect::<BTreeMap<_, _>>();
    assert_ne!(ids_by_state[&0], ids_by_state[&1]);
    assert_eq!(
        materials[ids_by_state[&0] as usize].texture,
        materials[ids_by_state[&1] as usize].texture
    );
    let keys = MaterialKeys::from_entries(ids.values().map(|&id| (id, "fixture")));
    let keys = super::super::pack_bindings::retain(
        keys,
        (&pack, root.path()),
        &ids,
        &materials,
        (&[], &[]),
        &[],
        &[],
    )
    .unwrap();
    for variant in 0..2 {
        assert_eq!(
            keys.material_source(ids_by_state[&variant])
                .unwrap()
                .variant,
            variant
        );
    }
}
