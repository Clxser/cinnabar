use super::*;

#[test]
fn rebudgeting_retains_admitted_resources_and_rejects_an_overcommitted_limit() {
    let mut scene = TerrainScene::new(128);
    let retained = ResourceKey::Mesh(AssetId([1; 32]), AssetId([2; 32]));
    scene
        .apply_allocations(BTreeMap::from([(retained, (64, 1))]), 0)
        .unwrap();
    scene.set_maximum_bytes(64).unwrap();
    assert_eq!(scene.resident_bytes(), 64);
    assert_eq!(scene.resources[&retained].references, 1);
    assert!(matches!(
        scene.set_maximum_bytes(63),
        Err(ArchiveError::Quota)
    ));
    assert_eq!(scene.maximum_bytes, 64);
    let additional = ResourceKey::World(AssetId([3; 32]));
    assert!(matches!(
        scene.apply_allocations(BTreeMap::from([(additional, (1, 1))]), 0),
        Err(ArchiveError::Quota)
    ));
    assert_eq!(scene.resident_bytes(), 64);
    assert!(!scene.resources.contains_key(&additional));
    scene.set_maximum_bytes(128).unwrap();
    scene
        .apply_allocations(BTreeMap::from([(additional, (64, 1))]), 0)
        .unwrap();
    assert_eq!(scene.resident_bytes(), 128);
    assert_eq!(scene.resources[&retained].references, 1);
}
