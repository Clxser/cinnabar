//! Resolves dimension mappings and chooses one complete program directory.

use crate::{programs::program_file, properties::ShaderProperties, source::PackSource};
use std::collections::{BTreeMap, BTreeSet};

/// Parsed dimension configuration and the directories containing known program stages.
#[derive(Clone, Debug)]
pub(crate) struct DimensionSelector {
    mappings: BTreeMap<String, String>,
    base: String,
    folders: BTreeSet<String>,
}

impl DimensionSelector {
    /// Resolves properties once, including duplicate keys and dimension identifier defaults.
    pub fn new(source: &PackSource) -> Self {
        let folders: BTreeSet<String> = source
            .files()
            .filter_map(|(path, _)| program_file(path).map(|(dir, _, _)| dir.to_string()))
            .collect();
        let properties = ShaderProperties::parse(
            source.text("dimension.properties").unwrap_or(""),
            &BTreeMap::new(),
        );
        let mut mappings = BTreeMap::new();
        let base = if properties.entries.is_empty() {
            for (id, folder) in [
                ("overworld", "world0"),
                ("the_nether", "world-1"),
                ("the_end", "world1"),
            ] {
                mappings.insert(normalize_id(id), folder.to_string());
            }
            if folders.contains("world0") {
                "world0".to_string()
            } else {
                String::new()
            }
        } else {
            // Property replacement keeps the key's original position in the file.
            let mut entries: Vec<(&str, &str)> = Vec::new();
            for (key, value) in &properties.entries {
                if let Some(entry) = entries.iter_mut().find(|entry| entry.0 == key) {
                    entry.1 = value;
                } else {
                    entries.push((key, value));
                }
            }
            for (key, value) in entries {
                if let Some(folder) = key.strip_prefix("dimension.") {
                    for id in value.split_whitespace() {
                        mappings.insert(normalize_id(id), folder.to_string());
                    }
                }
            }
            mappings.remove("*").unwrap_or_default()
        };
        Self {
            mappings,
            base,
            folders,
        }
    }

    /// Returns the configured directory for an identifier, or the configured base set.
    pub fn folder(&self, dimension: &str) -> String {
        let configured = self
            .mappings
            .get(&normalize_id(dimension))
            .unwrap_or(&self.base);
        self.program_folder(configured).to_string()
    }

    /// Selects a complete directory once; an absent override uses the base program set.
    pub fn program_folder<'a>(&'a self, requested: &'a str) -> &'a str {
        if requested.is_empty() || self.folders.contains(requested) {
            requested
        } else {
            &self.base
        }
    }
}

/// Applies the default namespace while preserving the special wildcard identifier.
fn normalize_id(id: &str) -> String {
    if id == "*" || id.contains(':') {
        id.to_string()
    } else {
        format!("minecraft:{id}")
    }
}
