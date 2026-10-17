//! The dense Bedrock `mc_Entity` table composed from a pack's block.properties and the
//! pinned Bedrock → Java block-state mapping.

use assets::facts::JavaBlockMapping;
use shaderpack::{IdMaps, UNMAPPED_BLOCK_ID};
use std::collections::BTreeMap;

/// Block-registry sequential ids whose Java state prints as `java`.
fn states(mapping: &JavaBlockMapping, java: &str) -> Vec<usize> {
    let found: Vec<usize> = mapping
        .block_states()
        .enumerate()
        .filter(|(_, state)| state.is_some_and(|state| state.to_string() == java))
        .map(|(index, _)| index)
        .collect();
    assert!(!found.is_empty(), "no Bedrock state maps to {java}");
    found
}

#[test]
fn bedrock_states_take_the_pack_ids_of_their_java_states() {
    let Ok(mapping) = JavaBlockMapping::pinned() else {
        panic!("the checked-in Java mapping validates");
    };
    let block = "block.31=birch_leaves minecraft:oak_leaves:persistent=false\n\
                 block.32=oak_leaves\n\
                 block.40=minecraft:oak_log:axis=y\n\
                 block.41=%logs\n\
                 block.50=water:level=0\n\
                 block.51=lantern:hanging=true:flavour=sweet\n";
    let maps = IdMaps::parse(Some(block), None, None, &BTreeMap::new());
    assert!(maps.diagnostics.is_empty(), "{:?}", maps.diagnostics);
    let table = maps.blocks.bedrock_block_ids(mapping);
    assert_eq!(table.len(), mapping.block_states().len());

    let id_of = |java: &str| {
        let ids: Vec<i32> = states(mapping, java)
            .into_iter()
            .map(|index| table[index])
            .collect();
        assert!(
            ids.windows(2).all(|pair| pair[0] == pair[1]),
            "{java}: {ids:?}"
        );
        ids[0]
    };
    assert_eq!(
        id_of("minecraft:oak_leaves[distance=1,persistent=false,waterlogged=false]"),
        31
    );
    assert_eq!(
        id_of("minecraft:oak_leaves[distance=1,persistent=true,waterlogged=false]"),
        32
    );
    assert_eq!(id_of("minecraft:oak_log[axis=y]"), 40);
    assert_eq!(id_of("minecraft:oak_log[axis=x]"), 41);
    assert_eq!(id_of("minecraft:spruce_log[axis=y]"), 41);
    assert_eq!(id_of("minecraft:water[level=0]"), 50);
    assert_eq!(id_of("minecraft:water[level=3]"), UNMAPPED_BLOCK_ID);
    assert_eq!(
        id_of("minecraft:lantern[hanging=true,waterlogged=false]"),
        51
    );
    assert_eq!(
        id_of("minecraft:lantern[hanging=false,waterlogged=false]"),
        UNMAPPED_BLOCK_ID
    );
    assert_eq!(id_of("minecraft:stone"), UNMAPPED_BLOCK_ID);
    let unmapped = mapping
        .block_states()
        .position(|state| state.is_none())
        .expect("some Bedrock states have no Java counterpart");
    assert_eq!(table[unmapped], UNMAPPED_BLOCK_ID);
}

#[test]
fn an_empty_map_leaves_every_state_unmapped() {
    let Ok(mapping) = JavaBlockMapping::pinned() else {
        panic!("the checked-in Java mapping validates");
    };
    let maps = IdMaps::parse(Some(""), None, None, &BTreeMap::new());
    assert!(
        maps.blocks
            .bedrock_block_ids(mapping)
            .iter()
            .all(|&id| id == UNMAPPED_BLOCK_ID)
    );
}
