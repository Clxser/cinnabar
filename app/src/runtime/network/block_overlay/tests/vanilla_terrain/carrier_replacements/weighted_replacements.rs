use super::*;

/// Builds a weighted cube whose selector and leaves initially use the carrier texture.
fn weighted_base() -> RuntimeAssets {
    let diagnostic = RuntimeAssets::diagnostic();
    let mut cube = super::super::super::super::diagnostic_visual();
    cube.kind = VisualKind::Cube;
    cube.support = VisualSupport::Exact;
    cube.flags = assets::BlockFlags::CUBE_GEOMETRY | assets::BlockFlags::OCCLUDES_FULL_FACE;
    cube.faces = [1; 6];
    let compiled = assets::CompiledAssets {
        visuals: vec![cube].into(),
        light_properties: vec![assets::LightProperties::OPAQUE_DARK].into(),
        hashed: Box::new([]),
        materials: vec![
            Material::unvaried(),
            Material {
                variation_start: 2,
                variation_count: 2,
                ..Material::unvaried()
            },
            Material {
                variation_weight: 0.25_f32.to_bits(),
                ..Material::unvaried()
            },
            Material {
                variation_weight: 0.75_f32.to_bits(),
                ..Material::unvaried()
            },
        ]
        .into(),
        model_templates: Box::new([]),
        model_quads: Box::new([]),
        animations: Box::new([]),
        animation_frames: Box::new([]),
        texture_pages: diagnostic.texture_pages().to_vec().into(),
        biomes: assets::CompiledBiomeAssets::diagnostic(),
        provenance: assets::BlobProvenance {
            source_manifest_sha256: [1; 32],
            block_registry_sha256: [2; 32],
            light_registry_sha256: [3; 32],
            biome_registry_sha256: [4; 32],
        },
    };
    RuntimeAssets::decode(&assets::encode_blob(&compiled).unwrap()).unwrap()
}

/// Retains the positional index even when both leaves name identical pixels.
fn weighted_keys(paths: [&str; 2]) -> MaterialKeys {
    MaterialKeys::from_entries([(1, "weighted"), (2, "weighted"), (3, "weighted")])
        .with_flipbook_selectors([(1, (0, 0)), (2, (0, 0)), (3, (0, 1))])
        .with_sources(
            [(
                "weighted".into(),
                json!({"textures":{"variations":[
                    {"path":paths[0],"weight":1}, {"path":paths[1],"weight":3}
                ]}}),
            )],
            [(1, paths[0]), (2, paths[0]), (3, paths[1])].map(|(id, path)| {
                (
                    id,
                    MaterialTextureSource {
                        key: "weighted".into(),
                        variant: 0,
                        path: path.into(),
                        raster: format!("{path}.png").into(),
                        baked_tint: None,
                    },
                )
            }),
            [],
        )
}

/// Confirms publication keeps the carrier selector range and each leaf's weight.
fn assert_weights(runtime: &RuntimeAssets) {
    assert_eq!(runtime.material(1).variation_start, 2);
    assert_eq!(runtime.material(1).variation_count, 2);
    assert_eq!(runtime.material(2).variation_weight, 0.25_f32.to_bits());
    assert_eq!(runtime.material(3).variation_weight, 0.75_f32.to_bits());
}

#[test]
fn repeated_raster_leaves_keep_their_recorded_flipbook_destination() {
    let base = weighted_base();
    let keys = weighted_keys(["textures/shared"; 2]);
    let blue = [10, 30, 220, 255];
    let view = timeline_pack(
        json!({}),
        json!([{"atlas_tile":"weighted","atlas_tile_variant":1,
            "flipbook_texture":"textures/strip","frames":[1,0],"ticks_per_frame":7}]),
        &[("textures/strip", 16, 32, [210, 20, 10, 255], blue)],
    );
    let compiled = compile(&view, &base, &keys);
    assert_eq!(compiled.overlay.material_overrides.len(), 1);
    assert_eq!(compiled.overlay.material_overrides[0].material, 3);
    let runtime = base.with_block_overlay(1, &compiled.overlay).unwrap();
    assert_eq!(runtime.material(1), base.material(1));
    assert_eq!(runtime.material(2), base.material(2));
    let leaf = runtime.material(3);
    assert_ne!(leaf.animation, assets::NO_ANIMATION);
    let animation = runtime.animations()[leaf.animation as usize];
    assert_eq!(
        (animation.atlas_index, animation.atlas_tile_variant),
        (0, 1)
    );
    assert_eq!(material_color(&runtime, 3), blue);
    assert_weights(&runtime);
}

#[test]
fn first_leaf_replacement_refreshes_the_selector_representative_and_preserves_weights() {
    let base = weighted_base();
    let keys = weighted_keys(["textures/first", "textures/second"]);
    let red = [210, 20, 10, 255];
    let blue = [10, 30, 220, 255];
    for animated in [false, true] {
        let view = timeline_pack(
            json!({}),
            if animated {
                json!([{"atlas_tile":"weighted","atlas_tile_variant":0,
                    "flipbook_texture":"textures/first","frames":[1,0],"ticks_per_frame":3}])
            } else {
                json!([])
            },
            &[(
                "textures/first",
                16,
                if animated { 32 } else { 16 },
                red,
                blue,
            )],
        );
        let compiled = compile(&view, &base, &keys);
        let runtime = base.with_block_overlay(1, &compiled.overlay).unwrap();
        let selector = runtime.material(1);
        let first = runtime.material(2);
        assert_eq!(selector.texture, first.texture);
        assert_eq!(selector.animation, first.animation);
        assert_eq!(selector.animation != assets::NO_ANIMATION, animated);
        assert_eq!(
            material_color(&runtime, 1),
            if animated { blue } else { red }
        );
        assert_eq!(runtime.material(3), base.material(3));
        assert_weights(&runtime);
        assert_eq!(base.material(1).texture.page(), 0);
    }
}
