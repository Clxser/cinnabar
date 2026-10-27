//! Shield compositions follow the admitted stack, selected base sheet and active pack.
use super::*;

/// Supplies original mask pixels without any installed game carriers.
fn catalog(mask: [u8; 4]) -> Arc<RuntimeEquipmentCatalog> {
    Arc::new(
        RuntimeEquipmentCatalog::from_parts(
            [7; 32],
            Vec::new(),
            vec![assets::EquipmentTexture {
                identifier: assets::item::shield::pattern_path("").into(),
                width: 2,
                height: 1,
                rgba8: [mask, [0; 4]].concat().into(),
            }],
        )
        .unwrap(),
    )
}

#[test]
fn shield_decoration_follows_selected_artwork_and_recovers_after_pack_reload() {
    let (fixture, _, mut item) = block_fixture();
    let (mut runtime, pages, _) = EquipmentRuntime::build(
        Arc::clone(&fixture.assets),
        Some(catalog([255; 4])),
        fixture.icons,
        None,
        None,
        ActorArtworkPages::default(),
    );
    let source = render_model::EquipmentRaster {
        width: 2,
        height: 1,
        rgba8: [10, 20, 30, 255, 40, 50, 60, 255].into(),
    };
    let (pages, locations) = pages.with_equipment_rasters(&[source]);
    let location = locations[0].unwrap();
    item.identifier = assets::gui_item::SHIELD_IDENTIFIER.into();
    assert!(runtime.shield_raster(&item, &pages, location).is_none());
    item.shield_pattern = Some(Arc::new(protocol::ItemShieldPattern {
        base: 15,
        layers: Arc::from([]),
    }));
    runtime.begin_shield_frame();
    let first = runtime.shield_raster(&item, &pages, location).unwrap();
    assert_eq!(&first.rgba8[..3], assets::banner::color_rgb(15));
    assert_eq!(&first.rgba8[4..], &[40, 50, 60, 255]);
    let reused = runtime.shield_raster(&item, &pages, location).unwrap();
    assert!(Arc::ptr_eq(&first.rgba8, &reused.rgba8));
    runtime.finish_shield_frame();
    runtime.begin_shield_frame();
    let next = runtime.shield_raster(&item, &pages, location).unwrap();
    assert!(Arc::ptr_eq(&first.rgba8, &next.rgba8));
    runtime.finish_shield_frame();
    runtime.set_pack_layer(Some((
        Arc::clone(&runtime.assets),
        Some(catalog([255, 0, 0, 255])),
        Vec::new(),
    )));
    let changed = runtime.shield_raster(&item, &pages, location).unwrap();
    assert_eq!(
        &changed.rgba8[..3],
        &[assets::banner::color_rgb(15)[0], 0, 0]
    );
    assert!(!Arc::ptr_eq(&first.rgba8, &changed.rgba8));
    runtime.begin_shield_frame();
    runtime.finish_shield_frame();
    let recovered = runtime.shield_raster(&item, &pages, location).unwrap();
    assert!(!Arc::ptr_eq(&changed.rgba8, &recovered.rgba8));
    assert_eq!(changed.rgba8, recovered.rgba8);
}
