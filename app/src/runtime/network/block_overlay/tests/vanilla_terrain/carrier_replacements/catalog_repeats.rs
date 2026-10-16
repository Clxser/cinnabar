use super::*;

#[test]
fn repeated_catalogs_and_bindings_leave_capacity_for_late_hd_replacements() {
    let count = 200;
    let base = carrier(&[[100, 120, 140, 255]], [1; 6], &vec![0; count + 1]);
    let terrain = (0..=count)
        .map(|index| {
            (
                format!("entry_{index}").into_boxed_str(),
                json!({"textures":format!("textures/entry_{index}")}),
            )
        })
        .collect::<Vec<_>>();
    let materials = (0..=count).map(|index| {
        (
            index as u32 + 1,
            MaterialTextureSource {
                key: format!("entry_{index}").into(),
                variant: 0,
                path: format!("textures/entry_{index}").into(),
                raster: format!("textures/entry_{index}.png").into(),
                baked_tint: None,
            },
        )
    });
    let mut blocks = keys().block_sources().to_vec();
    blocks.truncate(1);
    blocks[0].carried_fallback_keys = std::array::from_fn(|_| Some("entry_0".into()));
    let keys = MaterialKeys::from_entries(
        (0..=count).map(|index| (index as u32 + 1, format!("entry_{index}"))),
    )
    .with_sources(terrain.clone(), materials, blocks);
    let catalog = json!({"texture_data":terrain.into_iter().map(|(key, value)| (key.to_string(), value)).collect::<serde_json::Map<_, _>>()});
    let color = [20, 30, 200, 255];
    let raster = format!("textures/entry_{count}");
    for repeated_binding in [false, true] {
        let bindings = if repeated_binding {
            json!({"fixture":{"textures":"entry_0","carried_textures":"entry_0"}})
        } else {
            json!({})
        };
        let view = timeline_pack_with_bindings(
            catalog.clone(),
            json!([]),
            &[(&raster, 256, 256, color, color)],
            bindings,
        );
        let compiled = compile(&view, &base, &keys);
        assert_eq!(compiled.overlay.material_overrides.len(), 1);
        assert_eq!(
            compiled.overlay.material_overrides[0].material,
            count as u32 + 1
        );
        assert_eq!(compiled.overlay.textures.first().unwrap().layers, 2);
        assert!(compiled.vanilla_carried.is_empty());
        assert!(compiled.overlay.face_overrides.is_empty());
        let runtime = base.with_block_overlay(2, &compiled.overlay).unwrap();
        assert_eq!(material_color(&runtime, count as u32 + 1), color);
    }
}

#[test]
fn unchanged_variants_of_a_partially_changed_entry_keep_base_references() {
    let base = carrier(&[[100, 120, 140, 255]], [1; 6], &[0, 0]);
    let keys = keys();
    let view = stack(&[(
        json!({"texture_data":{"state":{"textures":["textures/base_a","textures/new"]}}}),
        json!({}),
        vec![("textures/new", [20, 30, 200, 255])],
    )]);
    let compiled = compile(&view, &base, &keys);
    assert_eq!(compiled.overlay.material_overrides.len(), 1);
    assert_eq!(compiled.overlay.material_overrides[0].material, 2);
    let runtime = base.with_block_overlay(2, &compiled.overlay).unwrap();
    assert_eq!(runtime.material(1), base.material(1));
    assert_eq!(material_color(&runtime, 2), [20, 30, 200, 255]);
}
