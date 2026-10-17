//! The `mc_Entity` table for Cinnabar's Bedrock block states: each state's Java block state
//! from the pinned mapping, looked up in the pack's [`BlockIdMap`].

use super::BlockIdMap;
use assets::facts::JavaBlockMapping;

/// The `mc_Entity` value of a block state no pack entry (or no Java state) matches.
pub const UNMAPPED_BLOCK_ID: i32 = -1;

impl BlockIdMap {
    /// The pack block id of every Bedrock block state, indexed by block-registry
    /// sequential id (`assets::RegistryRecord::sequential_id`, Cinnabar's dense block state
    /// index): `Vec<i32>` with one value per registry state, [`UNMAPPED_BLOCK_ID`] where the
    /// state has no Java counterpart or the pack assigns it no id. Ready to upload as the
    /// per-state `mc_Entity` lookup.
    pub fn bedrock_block_ids(&self, mapping: &JavaBlockMapping) -> Vec<i32> {
        let mut properties = Vec::new();
        mapping
            .block_states()
            .map(|state| {
                let Some(state) = state else {
                    return UNMAPPED_BLOCK_ID;
                };
                properties.clear();
                properties.extend(state.properties());
                self.block_id(state.name(), &properties, &|tag| state.in_tag(tag))
                    .unwrap_or(UNMAPPED_BLOCK_ID)
            })
            .collect()
    }
}
