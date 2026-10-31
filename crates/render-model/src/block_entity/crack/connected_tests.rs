use super::*;
use assets::{NetworkIdMode, RegistryRecord};
use std::sync::{Arc, OnceLock};

/// Compiles connected block families with original test artwork and their registered IDs.
fn fixture() -> &'static (Arc<RuntimeAssets>, Vec<RegistryRecord>) {
    static FIXTURE: OnceLock<(Arc<RuntimeAssets>, Vec<RegistryRecord>)> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let records: Vec<_> = assets::read_registry_for_protocol(
            assets::pinned_block_registry_bytes(),
            assets::active_content_registry_protocol(),
        )
        .unwrap()
        .into_iter()
        .filter(|record| {
            record.flags.contains(assets::BlockFlags::AIR)
                || matches!(
                    record.model_family,
                    assets::ModelFamily::Pane
                        | assets::ModelFamily::Fence
                        | assets::ModelFamily::Wall
                )
        })
        .collect();
        let blocks = records
            .iter()
            .map(|record| {
                (
                    record.name.strip_prefix("minecraft:").unwrap().to_owned(),
                    serde_json::json!({"textures":"test"}),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir_all(root.join("textures/blocks")).unwrap();
        std::fs::write(
            root.join("blocks.json"),
            serde_json::to_vec(&blocks).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("textures/terrain_texture.json"),
            r#"{"texture_data":{"test":{"textures":"textures/blocks/test"}}}"#,
        )
        .unwrap();
        std::fs::write(root.join("textures/flipbook_textures.json"), "[]").unwrap();
        image::RgbaImage::from_pixel(16, 16, image::Rgba([127, 127, 127, 255]))
            .save(root.join("textures/blocks/test.png"))
            .unwrap();
        let count = records
            .iter()
            .map(|record| record.sequential_id as usize + 1)
            .max()
            .unwrap();
        let compiled = pack_compiler::compile_pack(
            root,
            &records,
            &vec![assets::LightProperties::default(); count],
        )
        .unwrap();
        (
            Arc::new(RuntimeAssets::decode(&assets::encode_blob(&compiled).unwrap()).unwrap()),
            records,
        )
    })
}

/// Selects the wire identity without changing the registry's dense ID space.
fn id(record: &RegistryRecord, mode: NetworkIdMode) -> u32 {
    match mode {
        NetworkIdMode::Sequential => record.sequential_id,
        NetworkIdMode::Hashed => record.network_hash,
    }
}

/// Resolves and caches one model at a chunk corner with neighbors selected by mask.
fn connected_shape(
    cache: &mut CrackShapeCache,
    record: &RegistryRecord,
    mode: NetworkIdMode,
    mask: u32,
) -> Arc<[CrackQuad]> {
    let assets = &fixture().0;
    let block = [15, 7, -16];
    let neighbor = |position: [i32; 3]| {
        let delta: [i32; 3] = std::array::from_fn(|axis| position[axis] - block[axis]);
        let bit = match delta {
            [0, 0, -1] => 1,
            [1, 0, 0] => 2,
            [0, 0, 1] => 4,
            [-1, 0, 0] => 8,
            _ => panic!("only horizontal neighbors are needed"),
        };
        (mask & bit != 0).then_some(id(record, mode))
    };
    let templates = assets.model_templates_at(mode, id(record, mode), block, neighbor);
    let shape = cache.shape(assets, mode, Some(id(record, mode)), block, templates);
    assert_eq!(
        Some(shape.clone()),
        crack_shape_for_block(assets, mode, id(record, mode), block, neighbor)
    );
    let CrackShape::Quads(quads) = shape else {
        panic!("connected model surfaces")
    };
    quads
}

/// Finds the full surface extent without including overlay depth bias.
fn bounds(quads: &[CrackQuad]) -> ([f32; 3], [f32; 3]) {
    quads.iter().flat_map(|quad| quad.corners).fold(
        ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
        |(min, max), corner| {
            (
                std::array::from_fn(|axis| min[axis].min(corner[axis])),
                std::array::from_fn(|axis| max[axis].max(corner[axis])),
            )
        },
    )
}

#[test]
fn connected_cracks_cover_straight_corner_and_t_panes_and_bars() {
    let (assets, records) = fixture();
    for record in records
        .iter()
        .filter(|record| record.model_family == assets::ModelFamily::Pane)
    {
        for mode in [NetworkIdMode::Sequential, NetworkIdMode::Hashed] {
            let mut cache = CrackShapeCache::default();
            for (mask, min, max, count) in [
                (5, [0.4375, 0.0, 0.0], [0.5625, 1.0, 1.0], 14),
                (3, [0.4375, 0.0, 0.0], [1.0, 1.0, 0.5625], 14),
                (7, [0.4375, 0.0, 0.0], [1.0, 1.0, 1.0], 18),
            ] {
                let quads = connected_shape(&mut cache, record, mode, mask);
                assert_eq!(bounds(&quads), (min, max), "{} mask {mask}", record.name);
                assert_eq!(quads.len(), count);
                let base = assets
                    .resolve(mode, id(record, mode))
                    .model_template()
                    .unwrap();
                assert_eq!(
                    Some(CrackShape::Quads(quads)),
                    crack_shape_from_template(assets, base + mask, 0, [15, 7, -16])
                );
            }
            let original = connected_shape(&mut cache, record, mode, 5);
            let isolated = connected_shape(&mut cache, record, mode, 0);
            assert_eq!(
                bounds(&isolated),
                ([0.4375, 0.0, 0.4375], [0.5625, 1.0, 0.5625])
            );
            let restored = connected_shape(&mut cache, record, mode, 5);
            assert!(
                Arc::ptr_eq(&original, &restored),
                "unchanged connections reuse surfaces"
            );
        }
    }
}

#[test]
fn connected_cracks_include_fence_posts_and_every_connected_rail() {
    let (assets, records) = fixture();
    for record in records
        .iter()
        .filter(|record| record.model_family == assets::ModelFamily::Fence)
    {
        for mode in [NetworkIdMode::Sequential, NetworkIdMode::Hashed] {
            let mut cache = CrackShapeCache::default();
            let base = assets
                .resolve(mode, id(record, mode))
                .model_template()
                .unwrap();
            for mask in 0_u32..16 {
                let quads = connected_shape(&mut cache, record, mode, mask);
                let CrackShape::Quads(post) =
                    crack_shape_from_template(assets, base, 0, [15, 7, -16]).unwrap()
                else {
                    panic!("fence post")
                };
                let mut expected = post.to_vec();
                if mask != 0 {
                    let CrackShape::Quads(rails) =
                        crack_shape_from_template(assets, base + 1 + mask, 0, [15, 7, -16])
                            .unwrap()
                    else {
                        panic!("fence rails")
                    };
                    assert!(!rails.is_empty());
                    expected.extend_from_slice(&rails);
                }
                assert_eq!(&*quads, expected, "{} mask {mask}", record.name);
                let (min, max) = bounds(&quads);
                for (bit, axis, extent) in [
                    (1, 2, min[2]),
                    (2, 0, max[0]),
                    (4, 2, max[2]),
                    (8, 0, min[0]),
                ] {
                    if mask & bit != 0 {
                        assert_eq!(
                            extent,
                            if bit == 1 || bit == 8 { 0.0 } else { 1.0 },
                            "axis {axis}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn connected_cracks_keep_each_wall_states_surfaces() {
    let (assets, records) = fixture();
    let mut cache = CrackShapeCache::default();
    for record in records
        .iter()
        .filter(|record| record.model_family == assets::ModelFamily::Wall)
    {
        for mode in [NetworkIdMode::Sequential, NetworkIdMode::Hashed] {
            let runtime_id = id(record, mode);
            let block = [15, 7, -16];
            let templates = assets.model_templates_at(mode, runtime_id, block, |_| {
                panic!("walls use stored connections")
            });
            let expected = assets
                .resolve(mode, runtime_id)
                .model_template()
                .and_then(|template| crack_shape_from_template(assets, template, 0, block))
                .unwrap_or_default();
            assert_eq!(
                cache.shape(assets, mode, Some(runtime_id), block, templates),
                expected,
                "{} {}",
                record.name,
                record.canonical_state
            );
        }
    }
}
