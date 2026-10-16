//! Texture path selection inside catalog entries.

use super::{Value, normalize_texture_path};

/// A texture entry is a path, an object with `path`, or a variation list whose
/// first element is used.
pub(super) fn first_texture_path(value: &Value) -> Option<String> {
    let path = match value {
        Value::String(path) => path.as_str(),
        Value::Object(entry) => entry.get("path")?.as_str()?,
        Value::Array(entries) => return first_texture_path(entries.first()?),
        _ => return None,
    };
    let path = normalize_texture_path(path);
    (!path.is_empty()).then(|| path.to_owned())
}

/// Finds the first indexed image candidate without reading or decoding image payloads.
/// Only admitted cached pixels may reuse it; uncached candidates still require normal decoding.
pub fn indexed_image_candidate(view: &super::LayeredPackView, path: &str) -> Option<String> {
    let path = normalize_texture_path(path);
    if let Some(image) = indexed_image(view, path) {
        return Some(image);
    }
    let source = format!("{path}.texture_set.json");
    let bytes = view.read_capped(&source, super::MAX_TEXTURE_SET_BYTES)?;
    let root = super::parse_pack_json(&bytes)?;
    match root.get("minecraft:texture_set")?.get("color")? {
        Value::String(name) => {
            let name = normalize_texture_path(name);
            let sibling = path
                .rsplit_once('/')
                .map_or_else(|| name.to_owned(), |(dir, _)| format!("{dir}/{name}"));
            [sibling, name.to_owned()]
                .into_iter()
                .filter(|target| target != path)
                .find_map(|target| indexed_image(view, &target))
        }
        Value::Array(channels) if matches!(channels.len(), 3 | 4) => {
            Some(source.to_ascii_lowercase())
        }
        _ => None,
    }
}

/// Uses the decoder's extension order and indexed file sizes without loading pixels.
fn indexed_image(view: &super::LayeredPackView, path: &str) -> Option<String> {
    super::texture_image_paths(path)
        .into_iter()
        .find(|path| view.layers().any(|layer| layer.file_size(path).is_some()))
        .map(|path| path.to_ascii_lowercase())
}
