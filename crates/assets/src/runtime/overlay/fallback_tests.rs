use super::*;
use crate::material_keys::MaterialTextureSource;

#[test]
fn fallback_requires_the_requested_baked_tint_after_sidecar_roundtrip() {
    let tint = [32, 128, 48];
    let materials = [
        Material::unvaried(),
        Material {
            texture: TextureRef::new(0, 1).unwrap(),
            ..Material::unvaried()
        },
        Material {
            texture: TextureRef::new(0, 2).unwrap(),
            ..Material::unvaried()
        },
    ];
    let keys = MaterialKeys::from_entries([(1, "waterlily"), (2, "waterlily")])
        .with_aliases([("waterlily", "textures/blocks/waterlily")])
        .with_fixed_tints([("waterlily", tint)]);
    for copied_tint in [Some(tint), None, Some([48, 128, 32])] {
        let keys = keys.clone().with_sources(
            [],
            [(1, None), (2, copied_tint)].map(|(id, baked_tint)| {
                (
                    id,
                    MaterialTextureSource {
                        key: "waterlily".into(),
                        variant: 0,
                        path: "textures/blocks/waterlily".into(),
                        raster: "textures/blocks/waterlily.png".into(),
                        baked_tint,
                    },
                )
            }),
            [],
        );
        let keys = MaterialKeys::from_json(&keys.to_json(materials.len() as u32), materials.len())
            .unwrap();
        let fallback = VanillaFallback::new(&keys, &materials);
        assert_eq!(
            fallback.texture("waterlily", Some(tint)),
            (copied_tint == Some(tint)).then_some(materials[2].texture)
        );
        assert_eq!(
            fallback.texture("waterlily", None),
            Some(materials[1].texture)
        );
    }
    assert!(
        VanillaFallback::new(&keys, &materials)
            .texture("waterlily", Some(tint))
            .is_none()
    );
}
