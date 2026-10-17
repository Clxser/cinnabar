use super::*;
use flate2::{Compression, write::GzEncoder};
use std::io::Write;

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The manifest's archive cache, or `None` (with the missing-fixture notice) when absent.
fn pinned_archive() -> Option<std::path::PathBuf> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join(java_mapping::SOURCE_MANIFEST)).unwrap())
            .unwrap();
    let path = root().join(manifest["archive"]["cache"].as_str().unwrap());
    if path.is_file() {
        Some(path)
    } else {
        eprintln!(
            "missing fixture: {}; skipping Java mapping regeneration (registryc --kind java-mapping downloads it)",
            path.display()
        );
        None
    }
}

#[test]
fn java_mapping_regenerates_every_byte_from_the_pinned_archive() {
    let Some(archive) = pinned_archive() else {
        return;
    };
    let scratch = tempfile::tempdir().unwrap();
    let out = scratch.path().join(java_mapping::FILENAME);
    let summary = write(&root(), Some(&archive), &out).unwrap();
    assert!(
        summary.mapped_states * 10 >= summary.states * 9,
        "{summary:?}"
    );
    let expected = root().join(java_mapping::SOURCE_PATH);
    assert_eq!(
        std::fs::read(&out).unwrap(),
        std::fs::read(&expected).unwrap()
    );
    assert_eq!(
        std::fs::read(out.with_extension("sha256")).unwrap(),
        std::fs::read(expected.with_extension("sha256")).unwrap()
    );
}

#[test]
fn an_unpinned_archive_cannot_supply_the_mapping() {
    let scratch = tempfile::tempdir().unwrap();
    let archive = scratch.path().join("mappings.zip");
    std::fs::write(&archive, b"not the pinned archive").unwrap();
    let out = scratch.path().join("out.bin");
    assert!(write(&root(), Some(&archive), &out).is_err());
    assert!(!out.exists());
}

#[test]
fn the_inverse_prefers_the_same_name_then_a_dry_state_then_text_order() {
    assert_eq!(
        preference(
            "minecraft:air",
            &["minecraft:cave_air", "minecraft:air", "minecraft:void_air"]
        )
        .unwrap(),
        ("minecraft:air".to_owned(), String::new())
    );
    assert_eq!(
        preference(
            "minecraft:oak_stairs",
            &[
                "minecraft:oak_stairs[facing=east,half=top,shape=straight,waterlogged=true]",
                "minecraft:oak_stairs[facing=east,half=top,shape=straight,waterlogged=false]",
                "minecraft:oak_stairs[facing=east,half=top,shape=outer_left,waterlogged=false]",
            ]
        )
        .unwrap(),
        (
            "minecraft:oak_stairs".to_owned(),
            "facing=east,half=top,shape=outer_left,waterlogged=false".to_owned()
        )
    );
    assert_eq!(
        preference("minecraft:flowing_water", &["minecraft:water[level=3]"]).unwrap(),
        ("minecraft:water".to_owned(), "level=3".to_owned())
    );
    assert!(preference("minecraft:stone", &["minecraft:stone[level=1"]).is_err());
}

#[test]
fn registry_states_read_bytes_and_ints_as_integers() {
    assert_eq!(
        bedrock_state(
            r#"{"wall_post_bit":{"type":"byte","value":1},"facing":{"type":"string","value":"up"},"age":{"type":"int","value":7}}"#
        )
        .unwrap(),
        vec![
            ("age".to_owned(), Scalar::Int(7)),
            ("facing".to_owned(), Scalar::Text("up".to_owned())),
            ("wall_post_bit".to_owned(), Scalar::Int(1)),
        ]
    );
    assert!(bedrock_state(r#"{"age":{"type":"float","value":1.5}}"#).is_err());
}

#[test]
fn java_block_registry_order_follows_the_registrations() {
    let source = "public static final Block FIRST = register(new Block(\"first_block\", builder()));\n\
                  public static final Block SECOND = register(new CustomBlock(\"second_block\", builder()\n    .booleanState(LIT)));\n";
    assert_eq!(
        java_block_registry(source).unwrap(),
        ["minecraft:first_block", "minecraft:second_block"]
    );
    assert!(java_block_registry("register(new Block(builder()));").is_err());
}

/// Writes a big-endian NBT name.
fn name(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u16).to_be_bytes());
    out.extend_from_slice(text.as_bytes());
}

#[test]
fn network_tags_yield_block_tags_and_skip_other_registries() {
    let mut nbt = vec![10];
    name(&mut nbt, "");
    // An item registry with one int-array tag the reader must skip.
    nbt.push(10);
    name(&mut nbt, "minecraft:item");
    nbt.push(11);
    name(&mut nbt, "minecraft:things");
    nbt.extend_from_slice(&1_i32.to_be_bytes());
    nbt.extend_from_slice(&5_i32.to_be_bytes());
    nbt.push(0);
    nbt.push(10);
    name(&mut nbt, "minecraft:block");
    nbt.push(11);
    name(&mut nbt, "minecraft:glowing");
    nbt.extend_from_slice(&2_i32.to_be_bytes());
    nbt.extend_from_slice(&3_i32.to_be_bytes());
    nbt.extend_from_slice(&1_i32.to_be_bytes());
    nbt.push(9);
    name(&mut nbt, "minecraft:empty");
    nbt.push(0);
    nbt.extend_from_slice(&0_i32.to_be_bytes());
    nbt.push(0);
    nbt.push(0);
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&nbt).unwrap();
    let compressed = encoder.finish().unwrap();
    assert_eq!(
        nbt::block_tags(&compressed).unwrap(),
        vec![
            ("minecraft:glowing".to_owned(), vec![3, 1]),
            ("minecraft:empty".to_owned(), vec![]),
        ]
    );
    let mut truncated = GzEncoder::new(Vec::new(), Compression::default());
    truncated.write_all(&nbt[..nbt.len() - 3]).unwrap();
    assert!(nbt::block_tags(&truncated.finish().unwrap()).is_err());
}

#[test]
fn the_shipped_license_is_the_pinned_upstream_license() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join(java_mapping::SOURCE_MANIFEST)).unwrap())
            .unwrap();
    let shipped = std::fs::read(root().join("assets/licenses/GeyserMC-mappings-MIT.txt")).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&shipped)),
        manifest["license"]["sha256"].as_str().unwrap()
    );
}
