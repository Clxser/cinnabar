//! Layered flipbook descriptions, keyed by the complete atlas destination.

use std::collections::BTreeMap;

use assets::texture::flipbook::FlipbookSource;
use resource_pack::LayeredPackView;
use serde_json::Value;

use crate::pack_textures::{MAX_CATALOG_ENTRIES, parse_pack_json};

mod destinations;
pub use destinations::destinations;

type Selector = (Box<str>, u32, Option<u32>);

/// Overlays only supplied fields of matching selectors and retains unrelated base animations.
pub fn flipbooks(view: &LayeredPackView, base: &[FlipbookSource]) -> Vec<FlipbookSource> {
    let mut entries = base
        .iter()
        .take(MAX_CATALOG_ENTRIES)
        .cloned()
        .collect::<Vec<_>>();
    for bytes in view.read_layers("textures/flipbook_textures.json") {
        merge(&mut entries, &bytes);
    }
    entries
}

/// Applies a bounded list of valid descriptions; malformed patches leave lower entries usable.
fn merge(entries: &mut Vec<FlipbookSource>, bytes: &[u8]) {
    let mut indices = entries
        .iter()
        .enumerate()
        .map(|(index, source)| {
            (
                (
                    source.atlas_tile.clone(),
                    source.atlas_index,
                    (!source.all_variants).then_some(source.atlas_tile_variant),
                ),
                index,
            )
        })
        .collect::<BTreeMap<Selector, usize>>();
    let Some(Value::Array(layer)) = parse_pack_json(bytes) else {
        return;
    };
    for value in layer.into_iter().take(MAX_CATALOG_ENTRIES) {
        let Value::Object(mut fields) = value else {
            continue;
        };
        let Some(tile) = fields.get("atlas_tile").and_then(Value::as_str) else {
            continue;
        };
        let Some(index) = selector_index(fields.get("atlas_index")) else {
            continue;
        };
        let Some(variant) = selector_index(fields.get("atlas_tile_variant")) else {
            continue;
        };
        let variant = fields.contains_key("atlas_tile_variant").then_some(variant);
        let selector = (Box::<str>::from(tile), index, variant);
        let position = indices.get(&selector).copied();
        if entries.len() >= MAX_CATALOG_ENTRIES && position.is_none() {
            continue;
        }
        let lower = position
            .or_else(|| match variant {
                None | Some(0) => {
                    let other = (
                        selector.0.clone(),
                        index,
                        if variant.is_none() { Some(0) } else { None },
                    );
                    indices.get(&other).copied()
                }
                _ => None,
            })
            .map(|position| &entries[position]);
        fields.insert("all_variants".into(), Value::Bool(variant.is_none()));
        let mut merged = lower.map_or_else(serde_json::Map::new, |source| {
            serde_json::to_value(source)
                .expect("flipbook serializes")
                .as_object()
                .expect("flipbook object")
                .clone()
        });
        merged.extend(fields);
        if let Ok(mut source) = serde_json::from_value::<FlipbookSource>(Value::Object(merged)) {
            source.ticks_per_frame = source.ticks_per_frame.max(1);
            if !source.is_valid() {
                continue;
            }
            if let Some(position) = position {
                entries[position] = source;
            } else {
                indices.insert(selector, entries.len());
                entries.push(source);
            }
        }
    }
}

/// Reads a selector index with zero as its omitted value, rejecting malformed integers.
fn selector_index(value: Option<&Value>) -> Option<u32> {
    value.map_or(Some(0), |value| {
        value.as_u64().and_then(|value| u32::try_from(value).ok())
    })
}

#[cfg(test)]
mod tests;
