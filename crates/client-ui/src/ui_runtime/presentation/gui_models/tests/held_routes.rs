//! Ordinary held models use the same routes and geometry in previews and world hands.

use super::*;
use assets::{ItemVisualKey, RuntimeIconCatalog};
use player_preview::PreviewHeldPlacement;

/// Compiles original player hand bones and binds one item to a carried pane model.
fn pane_fixture() -> (RuntimeAssets, RuntimeEntityAssets, Arc<RuntimeIconCatalog>) {
    let mut compiled = pack_entity_compiler::compile_entity_pack(vec![
        ("entity/player.json".into(), br#"{"format_version":"1.10.0","minecraft:client_entity":{"description":{"identifier":"minecraft:player","geometry":{"default":"geometry.player"},"render_controllers":["controller.render.player"]}}}"#.to_vec()),
        ("models/player.json".into(), br#"{"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"geometry.player","texture_width":64,"texture_height":64},"bones":[{"name":"body"},{"name":"rightItem","parent":"body","pivot":[-6,12,0]},{"name":"leftItem","parent":"body","pivot":[6,12,0]}]}]}"#.to_vec()),
        ("render_controllers/player.json".into(), br#"{"format_version":"1.8.0","render_controllers":{"controller.render.player":{"geometry":"Geometry.default"}}}"#.to_vec()),
    ]).unwrap().unwrap().assets;
    compiled.block_visual_count = 3;
    compiled.item_visuals = vec![assets::ItemVisualDefinition {
        key: ItemVisualKey {
            identifier: "minecraft:glass_pane".into(),
            metadata: 0,
        },
        source: 0,
        route: ItemVisualDefinitionRoute::BlockItem {
            block_visual: assets::BlockVisualId(2),
        },
        grip: assets::item::grip::ItemGrip::Unresolved,
        glint: assets::item::glint::ItemGlint::Unresolved,
        fuel: assets::item::fuel::ItemFuel::Unresolved,
        durability: assets::item::durability::ItemDurability::Unresolved,
        armour: assets::item::armour::ItemArmour::Unresolved,
        first_person: assets::ItemDisplayTransform::identity(),
        third_person: assets::ItemDisplayTransform::identity(),
        dropped: assets::ItemDisplayTransform::identity(),
    }]
    .into();
    let entities = RuntimeEntityAssets::from_compiled(compiled.with_unresolved_item_use()).unwrap();
    let base = short_models::world(entities.source_manifest_sha256(), 256, 128);
    let mut block = assets::BlockVisual::diagnostic(
        assets::BlockFlags::empty(),
        assets::ContributorRole::Primary,
    );
    block.kind = assets::VisualKind::Model;
    block.support = assets::VisualSupport::Exact;
    block.faces = [1; 6];
    block.model_template = 0;
    let world = base
        .with_block_overlay(
            2,
            &assets::BlockOverlay {
                visuals: vec![block],
                light_properties: vec![assets::LightProperties::OPAQUE_DARK],
                model_templates: vec![assets::ModelTemplate {
                    quad_start: 0,
                    quad_count: 6,
                    flags: assets::MODEL_TEMPLATE_FLAG_PANE,
                }],
                model_quads: base.model_quads().to_vec(),
                materials: base.materials().to_vec(),
                base_texture_pages: base.texture_pages().len() as u32,
                ..Default::default()
            },
        )
        .unwrap();
    let icons = RuntimeIconCatalog::decode(
        &assets::encode_icon_catalog_with_blocks(
            entities.source_manifest_sha256(),
            &[assets::IconSprite {
                width: 16,
                height: 16,
                rgba8: [50, 90, 130, 200].repeat(256).into(),
            }],
            &[assets::IconEntry {
                identifier: "minecraft:glass_pane".into(),
                metadata: 0,
                sprite: 0,
            }],
            &[],
            &[assets::IconBlockModel {
                sprite: 0,
                visual: assets::BlockVisualId(2),
                held_sprite: None,
            }],
        )
        .unwrap(),
    )
    .unwrap();
    (world, entities, Arc::new(icons))
}

#[test]
fn player_preview_holds_the_carried_glass_plate_in_both_hands() {
    let (world, entities, icons) = pane_fixture();
    let mut preview = UiPresentationRuntime::with_hud_and_icons(
        ui_engine::test_support::fixture_font(),
        crate::test_support::fixture_hud(),
        Arc::clone(&icons),
    )
    .unwrap();
    preview.set_gui_models(&world, &entities).unwrap();
    let held = &preview.gui_models.sources.held[&entities.item_visuals()[0].key];
    assert!(
        !held.is_sprite(),
        "the paper doll must not extrude the inventory thumbnail"
    );
    let model = held.model(&icons).unwrap();
    assert!(
        model
            .placements
            .iter()
            .all(|placement| matches!(placement, PreviewHeldPlacement::Block))
    );
    assert_eq!(model.vertices.len(), 36);
    for vertex in &*model.vertices {
        assert_eq!(vertex.position[0].abs(), 1.0 / 16.0);
        assert_eq!(vertex.position[1].abs(), 0.5);
        assert_eq!(vertex.position[2].abs(), 0.5);
    }
    assert_ordinary_routes(&preview, &world, &entities, &icons);
}

/// Compares actual prepared preview geometry and sprite artwork with every ordinary world route.
fn assert_ordinary_routes(
    preview: &UiPresentationRuntime,
    world: &RuntimeAssets,
    entities: &RuntimeEntityAssets,
    icons: &RuntimeIconCatalog,
) {
    let blocks = render_model::equipment::blocks::prepare_held(Some(world), entities, icons);
    for entry in icons.entries() {
        let key = ItemVisualKey {
            identifier: entry.identifier.clone(),
            metadata: entry.metadata,
        };
        let definition = entities
            .item_visuals()
            .iter()
            .find(|definition| definition.key == key);
        if definition.is_some_and(|definition| {
            matches!(definition.route, ItemVisualDefinitionRoute::EmptyHand)
        }) {
            assert!(!preview.gui_models.sources.held.contains_key(&key));
            continue;
        }
        let held = preview
            .gui_models
            .sources
            .held
            .get(&key)
            .unwrap_or_else(|| panic!("missing preview for {key:?}"));
        let model = held
            .model(icons)
            .unwrap_or_else(|| panic!("empty held geometry for {key:?}"));
        let visual = definition.and_then(|definition| match definition.route {
            ItemVisualDefinitionRoute::BlockItem { block_visual } => Some(block_visual.0),
            _ => None,
        });
        let expected = visual.filter(|visual| blocks.by_visual.contains_key(visual));
        assert_eq!(
            !held.is_sprite(),
            expected.is_some(),
            "held route differs for {key:?}"
        );
        if let Some(visual) = expected {
            let cube;
            let vertices = if let Some(vertices) = blocks.models.get(&visual) {
                vertices
            } else {
                cube = render_model::textured_cube_vertices(
                    render_model::equipment::blocks::face_rects([0.0, 0.0, 1.0, 1.0]),
                );
                &cube
            };
            assert_eq!(
                &*model.vertices, vertices,
                "held geometry differs for {key:?}"
            );
            assert!(
                model
                    .placements
                    .iter()
                    .all(|placement| matches!(placement, PreviewHeldPlacement::Block))
            );
        } else {
            let sprite = icons
                .lookup_held_index(&entry.identifier, entry.metadata)
                .unwrap();
            let source = preview.icon_refs.as_deref().unwrap()[sprite];
            assert_eq!(
                (model.source.page, model.source.uv),
                (source.page, source.uv)
            );
        }
    }
}

#[test]
fn every_installed_item_has_the_same_ordinary_held_route_in_preview_and_world() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(assets::carriers::COMPILED_DIR);
    let read = |carrier: &assets::carriers::Carrier| {
        let path = root.join(carrier.output);
        std::fs::read(&path).map_err(|error| {
            assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "{}: {error}", path.display());
            eprintln!("skipping every_installed_item_has_the_same_ordinary_held_route_in_preview_and_world: missing {}; run make assets", path.display());
        })
    };
    let (Ok(world), Ok(entities), Ok(icons)) = (
        read(&assets::carriers::WORLD),
        read(&assets::carriers::ENTITY),
        read(&assets::carriers::ICON),
    ) else {
        return;
    };
    let world = RuntimeAssets::decode(&world).unwrap();
    let entities = RuntimeEntityAssets::decode(&entities).unwrap();
    let icons = Arc::new(RuntimeIconCatalog::decode(&icons).unwrap());
    let mut preview = UiPresentationRuntime::with_hud_and_icons(
        ui_engine::test_support::fixture_font(),
        crate::test_support::fixture_hud(),
        Arc::clone(&icons),
    )
    .unwrap();
    preview.set_gui_models(&world, &entities).unwrap();
    assert_ordinary_routes(&preview, &world, &entities, &icons);
}
