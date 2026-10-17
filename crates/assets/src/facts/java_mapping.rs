//! Java Edition identities of the pinned Bedrock registries, for Java shader packs: every block
//! state of the block registry to one Java block state, Java block tags, and Bedrock biome ids
//! to Java biome names. `registryc --kind java-mapping` generates the carrier from the pinned
//! GeyserMC mappings named by [`SOURCE_MANIFEST`].
//!
//! The carrier is optional: consumers treat a carrier that fails validation as "nothing
//! mapped" and keep running.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, sync::LazyLock};
use thiserror::Error;

pub const FILENAME: &str = "java-mapping-v2193.bin";
pub const SOURCE_PATH: &str = "crates/assets/data/java-mapping-v2193.bin";
/// The pinned GeyserMC source the generator reads.
pub const SOURCE_MANIFEST: &str = "assets/geyser-mappings-source.json";
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAGIC: &[u8; 8] = b"JAVAMAP1";
const SOURCE_BYTES: &[u8] = include_bytes!("../../data/java-mapping-v2193.bin");
const SOURCE_SHA256: &str = include_str!("../../data/java-mapping-v2193.sha256");
const BIOME_REGISTRY_BYTES: &[u8] = include_bytes!("../../data/biome-registry-v2193.bin");
const SOURCE_MANIFEST_BYTES: &[u8] =
    include_bytes!("../../../../assets/geyser-mappings-source.json");

/// A carrier error always names the failed validation.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("invalid Java mapping carrier ({0}); rebuild with registryc --kind java-mapping")]
pub struct JavaMappingError(String);

/// The serialized carrier content, exactly as the generator writes it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct JavaMappingPayload {
    pub protocol: u32,
    /// SHA-256 of the block registry whose sequential ids index `states`.
    pub block_registry_sha256: [u8; 32],
    /// SHA-256 of the biome registry whose ids `biomes` maps.
    pub biome_registry_sha256: [u8; 32],
    /// SHA-256 of the source manifest the carrier was generated from.
    pub source_manifest_sha256: [u8; 32],
    /// Java block ids (`namespace:path`), sorted and unique.
    pub blocks: Box<[Box<str>]>,
    /// Java property lists in block-state syntax without brackets (`axis=x,waterlogged=false`),
    /// keys sorted within each list; sorted and unique, the empty list included when used.
    pub property_sets: Box<[Box<str>]>,
    /// Per block-registry sequential id: the Java block index and property-list index, or
    /// `None` when the state has no Java counterpart.
    pub states: Box<[Option<(u16, u32)>]>,
    /// Sequential ids, ascending, of mapped states Geyser never produces: they carry the Java
    /// state of the produced state of the same Bedrock block sharing the most properties.
    pub approximate: Box<[u32]>,
    /// Java block tags (`namespace:path`, sorted, unique) with their member block indices,
    /// ascending; only blocks some state maps to are listed.
    pub tags: Box<[JavaBlockTag]>,
    /// Bedrock biome id to Java biome id, sorted by Bedrock id.
    pub biomes: Box<[JavaBiome]>,
}

/// A Java block tag id (`namespace:path`) and its member block indices, ascending.
pub type JavaBlockTag = (Box<str>, Box<[u16]>);

/// A Bedrock biome id and its Java biome id (`namespace:path`).
pub type JavaBiome = (u32, Box<str>);

/// One Java property list as `(key, value)` pairs sorted by key.
type PropertyList = Box<[(Box<str>, Box<str>)]>;

/// Decoded Java identities with per-block tag lookup.
#[derive(Clone, Debug)]
pub struct JavaBlockMapping {
    payload: JavaMappingPayload,
    properties: Box<[PropertyList]>,
    block_tags: Box<[Box<[u32]>]>,
}

/// One Java block state: a block id and its complete property list.
#[derive(Clone, Copy)]
pub struct JavaBlockState<'a> {
    mapping: &'a JavaBlockMapping,
    block: u16,
    properties: u32,
}

impl<'a> JavaBlockState<'a> {
    /// The Java block id, e.g. `minecraft:oak_leaves`.
    pub fn name(self) -> &'a str {
        &self.mapping.payload.blocks[usize::from(self.block)]
    }

    /// Every Java property as `(key, value)`, sorted by key.
    pub fn properties(self) -> impl ExactSizeIterator<Item = (&'a str, &'a str)> + 'a {
        self.mapping.properties[self.properties as usize]
            .iter()
            .map(|(key, value)| (key.as_ref(), value.as_ref()))
    }

    /// The value of one Java property, if the block has it.
    pub fn property(self, key: &str) -> Option<&'a str> {
        self.properties()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value)
    }

    /// The Java block tags (`namespace:path`) containing this block.
    pub fn tags(self) -> impl ExactSizeIterator<Item = &'a str> + 'a {
        let tags = &self.mapping.payload.tags;
        self.mapping.block_tags[usize::from(self.block)]
            .iter()
            .map(move |&tag| tags[tag as usize].0.as_ref())
    }

    /// Whether the block belongs to the Java block tag `tag` (`namespace:path`).
    pub fn in_tag(self, tag: &str) -> bool {
        self.tags().any(|name| name == tag)
    }
}

impl fmt::Display for JavaBlockState<'_> {
    /// Java block-state syntax: `minecraft:oak_log[axis=y]`, or the bare id without properties.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let properties = &self.mapping.payload.property_sets[self.properties as usize];
        if properties.is_empty() {
            f.write_str(self.name())
        } else {
            write!(f, "{}[{properties}]", self.name())
        }
    }
}

impl fmt::Debug for JavaBlockState<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JavaBlockState({self})")
    }
}

impl JavaBlockMapping {
    /// The checked-in carrier, validated once; an error means "nothing mapped".
    pub fn pinned() -> Result<&'static Self, &'static JavaMappingError> {
        static MAPPING: LazyLock<Result<JavaBlockMapping, JavaMappingError>> =
            LazyLock::new(|| {
                let mapping = JavaBlockMapping::validate_pinned(SOURCE_BYTES).and_then(|()| {
                    JavaBlockMapping::decode(
                        SOURCE_BYTES,
                        crate::active_content_registry_protocol(),
                    )
                });
                if let Err(error) = &mapping {
                    tracing::warn!(%error, "Java block and biome mapping unavailable");
                }
                mapping
            });
        MAPPING.as_ref()
    }

    /// Checks bytes against the reviewed carrier's external fingerprint.
    pub fn validate_pinned(bytes: &[u8]) -> Result<(), JavaMappingError> {
        ensure(bytes.len() <= MAX_BYTES, "carrier size")?;
        ensure(
            format!("{:x}", Sha256::digest(bytes)) == SOURCE_SHA256.trim(),
            "pinned carrier hash",
        )
    }

    /// Decodes a hash-sealed carrier bound to `protocol` and this checkout's registries.
    pub fn decode(bytes: &[u8], protocol: u32) -> Result<Self, JavaMappingError> {
        ensure(
            bytes.len() >= MAGIC.len() + 32 && bytes.len() <= MAX_BYTES,
            "carrier size",
        )?;
        let (content, checksum) = bytes.split_at(bytes.len() - 32);
        ensure(content.starts_with(MAGIC), "carrier format")?;
        ensure(
            Sha256::digest(content).as_slice() == checksum,
            "carrier checksum",
        )?;
        let payload: JavaMappingPayload = serde_json::from_slice(&content[MAGIC.len()..])
            .map_err(|error| JavaMappingError(error.to_string()))?;
        ensure(payload.protocol == protocol, "registry protocol")?;
        ensure(
            payload.block_registry_sha256
                == <[u8; 32]>::from(Sha256::digest(
                    crate::pinned_content::pinned_block_registry_bytes(),
                )),
            "block registry binding",
        )?;
        ensure(
            payload.biome_registry_sha256 == <[u8; 32]>::from(Sha256::digest(BIOME_REGISTRY_BYTES)),
            "biome registry binding",
        )?;
        ensure(
            payload.source_manifest_sha256
                == <[u8; 32]>::from(Sha256::digest(SOURCE_MANIFEST_BYTES)),
            "source manifest binding",
        )?;
        Self::from_payload(payload)
    }

    /// Encodes a payload exactly as [`Self::decode`] accepts it, with a trailing SHA-256 seal.
    pub fn encode(payload: &JavaMappingPayload) -> Result<Vec<u8>, JavaMappingError> {
        Self::from_payload(payload.clone())?;
        let mut bytes = MAGIC.to_vec();
        bytes.extend(
            serde_json::to_vec(payload).map_err(|error| JavaMappingError(error.to_string()))?,
        );
        bytes.extend_from_slice(&Sha256::digest(&bytes));
        ensure(bytes.len() <= MAX_BYTES, "carrier size")?;
        Ok(bytes)
    }

    /// Validates table ordering and every index, then builds the lookups.
    pub fn from_payload(payload: JavaMappingPayload) -> Result<Self, JavaMappingError> {
        ensure(
            !payload.blocks.is_empty() && payload.blocks.len() <= usize::from(u16::MAX),
            "block count",
        )?;
        ensure(strictly_sorted(&payload.blocks), "block order")?;
        ensure(
            payload.blocks.iter().all(|name| namespaced(name)),
            "block id syntax",
        )?;
        ensure(strictly_sorted(&payload.property_sets), "property order")?;
        let properties = payload
            .property_sets
            .iter()
            .map(|set| parse_properties(set))
            .collect::<Option<Box<[_]>>>()
            .ok_or_else(|| JavaMappingError("property syntax".into()))?;
        ensure(
            payload.states.iter().flatten().all(|&(block, set)| {
                usize::from(block) < payload.blocks.len() && (set as usize) < properties.len()
            }),
            "state index",
        )?;
        ensure(
            strictly_sorted(&payload.approximate)
                && payload
                    .approximate
                    .iter()
                    .all(|&id| payload.states.get(id as usize).is_some_and(Option::is_some)),
            "approximate states",
        )?;
        ensure(
            payload.tags.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "tag order",
        )?;
        let mut block_tags = vec![Vec::new(); payload.blocks.len()];
        for (index, (name, members)) in payload.tags.iter().enumerate() {
            ensure(namespaced(name), "tag id syntax")?;
            ensure(strictly_sorted(members), "tag member order")?;
            for &member in members.iter() {
                let tags = block_tags
                    .get_mut(usize::from(member))
                    .ok_or_else(|| JavaMappingError("tag member index".into()))?;
                tags.push(index as u32);
            }
        }
        ensure(
            payload.biomes.windows(2).all(|pair| pair[0].0 < pair[1].0)
                && payload.biomes.iter().all(|(_, name)| namespaced(name)),
            "biome table",
        )?;
        Ok(Self {
            payload,
            properties,
            block_tags: block_tags.into_iter().map(Vec::into_boxed_slice).collect(),
        })
    }

    /// The Java block state of a block-registry sequential id.
    pub fn java_block_state(&self, bedrock_state: u32) -> Option<JavaBlockState<'_>> {
        let (block, properties) = (*self.payload.states.get(bedrock_state as usize)?)?;
        Some(JavaBlockState {
            mapping: self,
            block,
            properties,
        })
    }

    /// Every block-registry state in sequential-id order, `None` where unmapped.
    pub fn block_states(&self) -> impl ExactSizeIterator<Item = Option<JavaBlockState<'_>>> {
        self.payload.states.iter().map(move |state| {
            state.map(|(block, properties)| JavaBlockState {
                mapping: self,
                block,
                properties,
            })
        })
    }

    /// Whether a mapped state took the Java state of a similar state of its Bedrock block
    /// because Geyser maps no Java state to it exactly.
    pub fn is_approximate(&self, bedrock_state: u32) -> bool {
        self.payload
            .approximate
            .binary_search(&bedrock_state)
            .is_ok()
    }

    /// The Java biome id of a Bedrock biome id.
    pub fn java_biome(&self, bedrock_biome: u32) -> Option<&str> {
        let biomes = &self.payload.biomes;
        biomes
            .binary_search_by_key(&bedrock_biome, |(id, _)| *id)
            .ok()
            .map(|index| biomes[index].1.as_ref())
    }

    /// The decoded payload.
    pub fn payload(&self) -> &JavaMappingPayload {
        &self.payload
    }
}

/// The Java block state of a block-registry sequential id, from the pinned carrier.
pub fn java_block_state(bedrock_state: u32) -> Option<JavaBlockState<'static>> {
    JavaBlockMapping::pinned()
        .ok()?
        .java_block_state(bedrock_state)
}

/// The Java biome id of a Bedrock biome id, from the pinned carrier.
pub fn java_biome(bedrock_biome: u32) -> Option<&'static str> {
    JavaBlockMapping::pinned().ok()?.java_biome(bedrock_biome)
}

/// Splits `key=value,...` into pairs; rejects empty parts and unsorted or repeated keys.
fn parse_properties(set: &str) -> Option<PropertyList> {
    if set.is_empty() {
        return Some(Box::default());
    }
    let pairs = set
        .split(',')
        .map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (!key.is_empty() && !value.is_empty() && !value.contains('='))
                .then(|| (Box::from(key), Box::from(value)))
        })
        .collect::<Option<Box<[(Box<str>, Box<str>)]>>>()?;
    pairs
        .windows(2)
        .all(|pair| pair[0].0 < pair[1].0)
        .then_some(pairs)
}

fn namespaced(name: &str) -> bool {
    name.split_once(':')
        .is_some_and(|(namespace, path)| !namespace.is_empty() && !path.is_empty())
}

fn strictly_sorted<T: Ord>(items: &[T]) -> bool {
    items.windows(2).all(|pair| pair[0] < pair[1])
}

fn ensure(condition: bool, what: &str) -> Result<(), JavaMappingError> {
    if condition {
        Ok(())
    } else {
        Err(JavaMappingError(what.to_owned()))
    }
}

#[cfg(test)]
mod tests;
