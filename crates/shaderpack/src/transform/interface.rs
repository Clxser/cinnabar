//! Location and binding numbering shared by a program's stages.

use crate::programs::StageKind;
use crate::reflection::{FIRST_TEXTURE_BINDING, LOOSE_UNIFORM_BINDING};
use std::collections::BTreeMap;

/// One stage-local varying awaiting program-wide location allocation.
struct Varying {
    symbol: String,
    size: u32,
    explicit: Option<u32>,
}

/// Stage-local varying declarations and program-wide resource bindings.
#[derive(Default)]
pub(crate) struct ProgramInterface {
    varyings: BTreeMap<(StageKind, String), Varying>,
    textures: BTreeMap<String, u32>,
    images: BTreeMap<String, u32>,
    uniform_blocks: BTreeMap<String, u32>,
    storage: BTreeMap<String, u32>,
}

impl ProgramInterface {
    /// Registers a varying before allocation so later explicit locations reserve their slots.
    pub(super) fn varying(
        &mut self,
        stage: StageKind,
        name: &str,
        size: u32,
        explicit: Option<u32>,
    ) -> String {
        let next = self.varyings.len();
        let varying = self
            .varyings
            .entry((stage, name.to_string()))
            .or_insert_with(|| Varying {
                symbol: format!("__shaderpack_location_{next}"),
                size,
                explicit,
            });
        varying.symbol.clone()
    }

    /// Allocates non-overlapping implicit locations after reserving every explicit range.
    pub(crate) fn location_defines(&self) -> Result<String, String> {
        let mut used = Vec::new();
        for varying in self.varyings.values() {
            if let Some(start) = varying.explicit {
                let end = start
                    .checked_add(varying.size)
                    .ok_or("varying location overflow")?;
                used.push(start..end);
            }
        }
        let mut implicit = BTreeMap::new();
        let mut definitions = String::new();
        for ((stage, name), varying) in &self.varyings {
            let other_stage = match stage {
                StageKind::Vertex => StageKind::Fragment,
                _ => StageKind::Vertex,
            };
            let peer = self.varyings.get(&(other_stage, name.clone()));
            // Explicit declarations own their locations independently of names. A name
            // links stages only when at least one declaration needs an implicit location.
            let fixed = varying
                .explicit
                .or_else(|| peer.and_then(|peer| peer.explicit));
            if varying.explicit.is_none() && peer.is_some_and(|peer| peer.size != varying.size) {
                return Err(format!(
                    "stages disagree on varying `{name}` location count"
                ));
            }
            let start = if let Some(explicit) = fixed {
                explicit
            } else if let Some(start) = implicit.get(name) {
                *start
            } else {
                let mut start = 0u32;
                loop {
                    let end = start
                        .checked_add(varying.size)
                        .ok_or("varying location overflow")?;
                    match used
                        .iter()
                        .find(|range| start < range.end && range.start < end)
                    {
                        Some(range) => start = range.end,
                        None => {
                            used.push(start..end);
                            implicit.insert(name, start);
                            break start;
                        }
                    }
                }
            };
            definitions.push_str(&format!("#define {} {start}\n", varying.symbol));
        }
        Ok(definitions)
    }

    /// Binding of sampled texture `name`, after the two shared samplers.
    pub(super) fn texture_binding(&mut self, name: &str) -> u32 {
        let next = FIRST_TEXTURE_BINDING + self.textures.len() as u32;
        *self.textures.entry(name.to_string()).or_insert(next)
    }

    /// Binding of storage image `name`.
    pub(super) fn image_binding(&mut self, name: &str) -> u32 {
        let next = self.images.len() as u32;
        *self.images.entry(name.to_string()).or_insert(next)
    }

    /// Binding of pack uniform block `name`, after the loose-uniform block.
    pub(super) fn uniform_block_binding(&mut self, name: &str) -> u32 {
        let next = LOOSE_UNIFORM_BINDING + 1 + self.uniform_blocks.len() as u32;
        *self.uniform_blocks.entry(name.to_string()).or_insert(next)
    }

    /// Binding of storage block `name`: the pack's own `binding`, else the next free one.
    pub(super) fn storage_binding(&mut self, name: &str, declared: Option<u32>) -> u32 {
        let next = self.storage.values().max().map_or(0, |m| m + 1);
        *self
            .storage
            .entry(name.to_string())
            .or_insert(declared.unwrap_or(next))
    }
}
