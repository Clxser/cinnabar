//! The local server's generator catalog, as defined by its shared JSON schema.

use serde::{Deserialize, Serialize};

use super::world::Generator;

/// One terrain option advertised by the selected local server binary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratorInfo {
    pub id: Generator,
    pub label: String,
}

/// Parses a nonempty catalog with unique, nonempty ids and labels.
pub fn parse_catalog(bytes: &[u8]) -> Result<Vec<GeneratorInfo>, String> {
    let entries: Vec<GeneratorInfo> = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if entries.is_empty() {
        return Err("the local server advertises no world generators".into());
    }
    let mut ids = std::collections::HashSet::new();
    for entry in &entries {
        if entry.id.as_str().is_empty() || entry.label.is_empty() || !ids.insert(entry.id.as_str())
        {
            return Err("invalid or duplicate local server generator".into());
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_catalog_conformance() {
        #[derive(Deserialize)]
        struct Case {
            name: String,
            json: String,
            accepted: bool,
        }
        let cases: Vec<Case> = serde_json::from_str(include_str!(
            "../../../../tools/localserver/contract/conformance.json"
        ))
        .unwrap();
        for case in cases {
            assert_eq!(
                parse_catalog(case.json.as_bytes()).is_ok(),
                case.accepted,
                "{}",
                case.name
            );
        }
    }
}

/// Returns the server's checked example catalog for offline UI fixtures.
#[cfg(any(test, feature = "test-support"))]
pub fn fixture_catalog() -> Vec<GeneratorInfo> {
    parse_catalog(include_bytes!(
        "../../../../tools/localserver/contract/catalog.example.json"
    ))
    .expect("valid server catalog fixture")
}
