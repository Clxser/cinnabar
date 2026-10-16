//! Atlas destination bounds retained without importing image pixels.

use std::collections::HashMap;

use assets::material_keys::MaterialKeys;
use resource_pack::LayeredPackView;
use serde_json::Value;

use crate::pack_textures::{MAX_CATALOG_ENTRIES, base_terrain_catalog, parse_pack_json};

/// Overlays each terrain key's atlas indices and UV-variant counts for flipbook creation.
pub fn destinations(
    view: &LayeredPackView,
    base: Option<&MaterialKeys>,
) -> HashMap<String, Vec<u32>> {
    let mut counts = base_terrain_catalog()
        .into_keys()
        .map(|key| (key, vec![1]))
        .collect::<HashMap<_, _>>();
    if let Some(base) = base {
        counts.extend(
            base.flipbook_destinations()
                .map(|(key, counts)| (key.to_owned(), counts.to_vec())),
        );
        for (key, entry) in base.terrain_entries() {
            if let Some(variants) = entry_counts(entry) {
                counts.insert(key.to_owned(), variants);
            }
        }
    }
    for bytes in view.read_layers("textures/terrain_texture.json") {
        let Some(Value::Object(data)) =
            parse_pack_json(&bytes).map(|mut root| root["texture_data"].take())
        else {
            continue;
        };
        for (key, entry) in data {
            if counts.len() >= MAX_CATALOG_ENTRIES && !counts.contains_key(&key) {
                continue;
            }
            if let Some(variants) = entry_counts(&entry) {
                counts.insert(key, variants);
            }
        }
    }
    counts
}

/// Counts only fully admitted terrain entries so malformed layers keep lower destination bounds.
fn entry_counts(entry: &Value) -> Option<Vec<u32>> {
    crate::pack_textures::terrain::parse_entry(entry)?;
    let values = match &entry["textures"] {
        Value::Array(values) => values.as_slice(),
        value => std::slice::from_ref(value),
    };
    values.iter().map(uv_count).collect()
}

/// Counts valid path alternatives at one state-selected atlas index.
fn uv_count(value: &Value) -> Option<u32> {
    if let Some(variations) = value.get("variations") {
        let variations = variations.as_array()?;
        if variations.is_empty()
            || variations.len() > MAX_CATALOG_ENTRIES
            || !variations.iter().all(has_path)
        {
            return None;
        }
        Some(variations.len() as u32)
    } else {
        has_path(value).then_some(1)
    }
}

/// Accepts a nonempty string or object image route without decoding it.
fn has_path(value: &Value) -> bool {
    value
        .as_str()
        .or_else(|| value.get("path").and_then(Value::as_str))
        .is_some_and(|path| !path.trim().is_empty())
}
