use super::*;
use crate::{
    EntityAssetKind, EntityGeometry, EntityGeometryBone, EntityGeometryCube, EntityGeometryScalar,
    EntityGeometryUv,
};

#[test]
fn startup_loads_valid_entity_catalog_above_eight_megabytes() {
    let mut compiled = super::super::test_carriers::synthetic_entity_assets(1);
    compiled.sources[0].path = "models/entity/fixture.geo.json".into();
    compiled.symbols[0].kind = EntityAssetKind::Geometry;
    let zero = EntityGeometryScalar::new(0.0).unwrap();
    let cube = EntityGeometryCube {
        origin: [zero; 3],
        size: [zero; 3],
        pivot: [zero; 3],
        rotation: [zero; 3],
        uv: EntityGeometryUv::Box([zero; 2]),
        inflate: zero,
        mirror: false,
    };
    let mut symbols = Vec::new();
    let mut geometries = Vec::new();
    for index in 0..3 {
        let identifier = format!("geometry.fixture_{index}");
        let mut symbol = compiled.symbols[0].clone();
        symbol.identifier = identifier.clone().into();
        symbols.push(symbol);
        geometries.push(EntityGeometry {
            identifier: identifier.into(),
            inherits: None,
            source_index: 0,
            texture_width: 16,
            texture_height: 16,
            animation_no_head_bob: false,
            visible_bounds: None,
            bones: Box::from([EntityGeometryBone {
                name: "root".into(),
                parent: None,
                pivot: None,
                rotation: None,
                bind_pose_rotation: None,
                mirror: None,
                inflate: None,
                never_render: None,
                reset: None,
                binding: None,
                locators: Box::new([]),
                texture_meshes: Box::new([]),
                cubes: vec![cube.clone(); crate::MAX_ENTITY_GEOMETRY_CUBES].into(),
            }]),
        });
    }
    compiled.symbols = symbols.into();
    compiled.geometries = geometries.into();
    let compiled = compiled
        .with_unresolved_item_use()
        .with_unresolved_seats()
        .with_unresolved_interactions();
    let bytes = crate::encode_entity_blob(&compiled).unwrap();
    assert!(bytes.len() > 8 * 1024 * 1024);
    let directory = tempfile::tempdir().unwrap();
    let world = directory.path().join("world.mcbe");
    std::fs::write(entity_asset_path(&world), bytes).unwrap();
    let loaded = load_entity_assets(&world).unwrap();
    assert_eq!(loaded.runtime().geometries(), compiled.geometries.as_ref());
}

#[test]
fn startup_rejects_entity_catalog_beyond_the_encoded_format_limit() {
    let directory = tempfile::tempdir().unwrap();
    let world = directory.path().join("world.mcbe");
    File::create(entity_asset_path(&world))
        .unwrap()
        .set_len(crate::entity::MAX_ENTITY_BLOB_BYTES as u64 + 1)
        .unwrap();
    assert!(matches!(load_entity_assets(&world),
        Err(AssetStartupError::EntityAssetsTooLarge { max_bytes, .. })
            if max_bytes == crate::entity::MAX_ENTITY_BLOB_BYTES as u64));
}
