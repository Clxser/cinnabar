use super::*;
use crate::{read_biome_registry, read_registry_for_protocol};

fn mapping() -> &'static JavaBlockMapping {
    JavaBlockMapping::pinned().expect("the checked-in Java mapping validates")
}

/// The sequential id of the registry state `name` whose canonical state has exactly these
/// property values (`byte`/`int` as numbers, `string` as text).
fn state(name: &str, properties: &[(&str, serde_json::Value)]) -> u32 {
    let records = read_registry_for_protocol(
        crate::pinned_content::pinned_block_registry_bytes(),
        crate::active_content_registry_protocol(),
    )
    .unwrap();
    records
        .iter()
        .find(|record| {
            let state: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(&record.canonical_state).unwrap();
            record.name.as_ref() == name
                && state.len() == properties.len()
                && properties
                    .iter()
                    .all(|(key, value)| state.get(*key).map(|typed| &typed["value"]) == Some(value))
        })
        .unwrap_or_else(|| panic!("no registry state {name} {properties:?}"))
        .sequential_id
}

fn java(name: &str, properties: &[(&str, serde_json::Value)]) -> String {
    mapping()
        .java_block_state(state(name, properties))
        .unwrap_or_else(|| panic!("{name} is unmapped"))
        .to_string()
}

#[test]
fn leaves_logs_water_and_stairs_map_to_their_java_states() {
    use serde_json::json;
    assert_eq!(
        java(
            "minecraft:oak_leaves",
            &[("persistent_bit", json!(0)), ("update_bit", json!(0))]
        ),
        "minecraft:oak_leaves[distance=1,persistent=false,waterlogged=false]"
    );
    assert_eq!(
        java(
            "minecraft:oak_leaves",
            &[("persistent_bit", json!(1)), ("update_bit", json!(0))]
        ),
        "minecraft:oak_leaves[distance=1,persistent=true,waterlogged=false]"
    );
    for axis in ["x", "y", "z"] {
        assert_eq!(
            java("minecraft:oak_log", &[("pillar_axis", json!(axis))]),
            format!("minecraft:oak_log[axis={axis}]")
        );
    }
    assert_eq!(
        java("minecraft:water", &[("liquid_depth", json!(0))]),
        "minecraft:water[level=0]"
    );
    assert_eq!(
        java("minecraft:flowing_water", &[("liquid_depth", json!(3))]),
        "minecraft:water[level=3]"
    );
    let stairs = mapping()
        .java_block_state(state(
            "minecraft:oak_stairs",
            &[
                ("minecraft:corner", json!("none")),
                ("upside_down_bit", json!(1)),
                ("weirdo_direction", json!(0)),
            ],
        ))
        .unwrap();
    assert_eq!(stairs.name(), "minecraft:oak_stairs");
    assert_eq!(stairs.property("facing"), Some("east"));
    assert_eq!(stairs.property("half"), Some("top"));
    assert_eq!(stairs.property("waterlogged"), Some("false"));
    assert_eq!(stairs.property("shape"), Some("straight"));
    assert_eq!(stairs.property("open"), None);
}

#[test]
fn waterloggable_blocks_map_to_their_dry_java_state() {
    use serde_json::json;
    assert_eq!(
        java("minecraft:lantern", &[("hanging", json!(1))]),
        "minecraft:lantern[hanging=true,waterlogged=false]"
    );
    assert_eq!(java("minecraft:air", &[]), "minecraft:air");
    let connections = [
        "minecraft:connection_east",
        "minecraft:connection_north",
        "minecraft:connection_south",
        "minecraft:connection_west",
    ]
    .map(|key| (key, serde_json::json!(0)));
    assert_eq!(
        java("minecraft:oak_fence", &connections),
        "minecraft:oak_fence[east=false,north=false,south=false,waterlogged=false,west=false]"
    );
}

#[test]
fn states_geyser_never_produces_take_the_closest_state_of_their_block() {
    use serde_json::json;
    let updating = state(
        "minecraft:oak_leaves",
        &[("persistent_bit", json!(1)), ("update_bit", json!(1))],
    );
    assert!(mapping().is_approximate(updating));
    assert_eq!(
        mapping().java_block_state(updating).unwrap().to_string(),
        "minecraft:oak_leaves[distance=1,persistent=true,waterlogged=false]"
    );
    let exact = state(
        "minecraft:oak_leaves",
        &[("persistent_bit", json!(1)), ("update_bit", json!(0))],
    );
    assert!(!mapping().is_approximate(exact));
}

#[test]
fn properties_are_sorted_and_tags_list_the_block() {
    use serde_json::json;
    let leaves = mapping()
        .java_block_state(state(
            "minecraft:oak_leaves",
            &[("persistent_bit", json!(0)), ("update_bit", json!(0))],
        ))
        .unwrap();
    let keys: Vec<&str> = leaves.properties().map(|(key, _)| key).collect();
    assert_eq!(keys, ["distance", "persistent", "waterlogged"]);
    assert!(leaves.in_tag("minecraft:leaves"));
    assert!(!leaves.in_tag("minecraft:logs"));
    let log = mapping()
        .java_block_state(state("minecraft:oak_log", &[("pillar_axis", json!("y"))]))
        .unwrap();
    assert!(log.in_tag("minecraft:logs"));
}

#[test]
fn unmapped_states_and_biomes_return_none() {
    let mapping = mapping();
    assert!(mapping.java_block_state(u32::MAX).is_none());
    assert!(
        mapping
            .java_block_state(mapping.payload().states.len() as u32)
            .is_none()
    );
    if let Some(index) = mapping.block_states().position(|state| state.is_none()) {
        assert!(mapping.java_block_state(index as u32).is_none());
    }
    assert!(mapping.java_biome(u32::MAX).is_none());
}

#[test]
fn dense_states_cover_the_whole_block_registry() {
    let records = read_registry_for_protocol(
        crate::pinned_content::pinned_block_registry_bytes(),
        crate::active_content_registry_protocol(),
    )
    .unwrap();
    assert_eq!(mapping().block_states().len(), records.len());
}

#[test]
fn biomes_map_to_the_java_biome_of_the_same_name() {
    let records = read_biome_registry(BIOME_REGISTRY_BYTES).unwrap();
    let id = |name: &str| {
        records
            .iter()
            .find(|record| record.name.as_ref() == name)
            .unwrap()
            .id
    };
    for name in ["minecraft:plains", "minecraft:the_end", "minecraft:river"] {
        assert_eq!(mapping().java_biome(id(name)), Some(name));
        assert_eq!(java_biome(id(name)), Some(name));
    }
}

#[test]
fn corrupt_or_unpinned_bytes_are_rejected() {
    let protocol = crate::active_content_registry_protocol();
    let mut corrupt = SOURCE_BYTES.to_vec();
    corrupt[MAGIC.len() + 4] ^= 1;
    assert!(JavaBlockMapping::decode(&corrupt, protocol).is_err());
    assert!(JavaBlockMapping::validate_pinned(&corrupt).is_err());
    assert!(JavaBlockMapping::decode(SOURCE_BYTES, protocol + 1).is_err());
    assert!(JavaBlockMapping::decode(b"JAVAMAP1", protocol).is_err());
}

#[test]
fn payload_validation_rejects_bad_order_and_indices() {
    let valid = mapping().payload().clone();
    assert!(JavaBlockMapping::encode(&valid).is_ok());
    let mut unsorted = valid.clone();
    unsorted.blocks.swap(0, 1);
    assert!(JavaBlockMapping::from_payload(unsorted).is_err());
    let mut out_of_range = valid.clone();
    out_of_range.states[0] = Some((u16::MAX, 0));
    assert!(JavaBlockMapping::from_payload(out_of_range).is_err());
    let mut bad_properties = valid;
    bad_properties.property_sets = Box::from([Box::from("b=1,a=2")]);
    assert!(JavaBlockMapping::from_payload(bad_properties).is_err());
}
