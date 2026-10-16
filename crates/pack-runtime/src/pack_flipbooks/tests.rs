use super::*;
use assets::material_keys::MaterialKeys;

#[test]
fn layers_merge_fields_by_full_selector_and_keep_other_animations() {
    let mut entries = Vec::new();
    merge(&mut entries, br#"[
        {"atlas_tile":"water","flipbook_texture":"textures/water","frames":[2,0,1],"ticks_per_frame":7,"replicate":2,"blend_frames":false},
        {"atlas_tile":"water","atlas_index":1,"atlas_tile_variant":2,"flipbook_texture":"textures/other"},
        {"atlas_tile":"fire","flipbook_texture":"textures/fire"}
    ]"#);
    merge(
        &mut entries,
        br#"[{"atlas_tile":"water","ticks_per_frame":3}]"#,
    );
    let water = &entries[0];
    assert_eq!(water.texture_path.as_ref(), "textures/water");
    assert_eq!(water.frames.as_ref(), [2, 0, 1]);
    assert_eq!(water.ticks_per_frame, 3);
    assert_eq!(water.replicate, 2);
    assert!(!water.blend_frames);
    assert!(water.all_variants);
    assert_eq!(entries.len(), 3);
    let other = &entries[1];
    assert!(!other.all_variants);
    assert_eq!(other.texture_path.as_ref(), "textures/other");
}

#[test]
fn malformed_patches_preserve_lower_descriptions_and_empty_lists_keep_them() {
    let mut entries = Vec::new();
    merge(
        &mut entries,
        br#"[{"atlas_tile":"ore","flipbook_texture":"textures/ore","ticks_per_frame":2}]"#,
    );
    let expected = entries.clone();
    for layer in [
        br#"[{"atlas_tile":"ore","replicate":0}]"#.as_slice(),
        br#"[{"atlas_tile":"ore","atlas_index":-1}]"#,
        br#"[{"atlas_tile":"ore","frames":[-1]}]"#,
        br#"[{"atlas_tile":"ore","frames":[0,1],"ticks_per_frame":4294967295}]"#,
        b"[]",
        b"{}",
        b"invalid",
    ] {
        merge(&mut entries, layer);
        assert_eq!(entries, expected);
    }
}

#[test]
fn zero_tick_duration_is_clamped_before_timeline_validation() {
    let mut entries = Vec::new();
    merge(
        &mut entries,
        br#"[{"atlas_tile":"tile","flipbook_texture":"textures/tile","ticks_per_frame":0}]"#,
    );
    assert_eq!(entries[0].ticks_per_frame, 1);
}

#[test]
fn matching_field_overlays_do_not_reorder_creation_and_new_selectors_append() {
    let mut entries = Vec::new();
    merge(
        &mut entries,
        br#"[
        {"atlas_tile":"tile","atlas_tile_variant":9,"flipbook_texture":"textures/first"},
        {"atlas_tile":"tile","atlas_tile_variant":4,"flipbook_texture":"textures/second"}
    ]"#,
    );
    merge(
        &mut entries,
        br#"[
        {"atlas_tile":"tile","atlas_tile_variant":9,"ticks_per_frame":7},
        {"atlas_tile":"tile","atlas_tile_variant":2,"flipbook_texture":"textures/third"}
    ]"#,
    );
    assert_eq!(
        entries
            .iter()
            .map(|source| source.atlas_tile_variant)
            .collect::<Vec<_>>(),
        [9, 4, 2]
    );
    assert_eq!(entries[0].ticks_per_frame, 7);
}

#[test]
fn destination_bounds_cover_unused_indices_and_weighted_uv_sets() {
    let base = MaterialKeys::from_entries([(1, "tile")])
        .with_aliases([("tile", "textures/base")])
        .with_flipbook_destinations([("tile", [2, 3])]);
    let view = LayeredPackView::new(crate::test_support::stack(&[("textures/terrain_texture.json", br#"{
        "texture_data":{"tile":{"textures":["textures/one", {"variations":["textures/two", {"path":"textures/three"}]}]}}
    }"#)]));
    assert_eq!(destinations(&view, Some(&base))["tile"], [1, 2]);
}

#[test]
fn explicit_zero_patch_keeps_the_wildcard_and_inherits_its_unsupplied_fields() {
    let mut entries = Vec::new();
    merge(
        &mut entries,
        br#"[{
        "atlas_tile":"tile", "flipbook_texture":"textures/base", "ticks_per_frame":7,
        "frames":[1,0], "replicate":2, "blend_frames":false
    }]"#,
    );
    let wildcard = entries[0].clone();
    merge(&mut entries, br#"[{
        "atlas_tile":"tile", "atlas_tile_variant":0, "flipbook_texture":"textures/upper", "ticks_per_frame":3
    }]"#);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0], wildcard);
    assert!(entries[0].all_variants);
    assert!(!entries[1].all_variants);
    assert_eq!(entries[1].texture_path.as_ref(), "textures/upper");
    assert_eq!(entries[1].ticks_per_frame, 3);
    assert_eq!(entries[1].frames, wildcard.frames);
    assert_eq!(entries[1].replicate, wildcard.replicate);
    assert_eq!(entries[1].blend_frames, wildcard.blend_frames);
}
