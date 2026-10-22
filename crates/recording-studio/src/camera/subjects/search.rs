//! A compiled Unicode-lowercase substring matcher avoids allocation while scanning recorded subjects.

use super::*;
use crate::{StudioError, error::limit};
use std::sync::Arc;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraSubjectSearch {
    text: Arc<str>,
    folded: Arc<[char]>,
    prefixes: Arc<[usize]>,
}

impl CameraSubjectSearch {
    /// Query admission shares the recorded-name budget; compile only when input changes.
    pub fn new(text: &str) -> Result<Self, StudioError> {
        limit(
            "camera subject search bytes",
            text.len(),
            super::super::MAX_TARGET_NAME_BYTES,
        )?;
        if text.chars().any(char::is_control) {
            return Err(StudioError::Invalid("subject search must be one line"));
        }
        let folded: Vec<_> = text.trim().chars().flat_map(char::to_lowercase).collect();
        let mut prefixes = vec![0; folded.len()];
        let mut matched = 0;
        for index in 1..folded.len() {
            while matched > 0 && folded[index] != folded[matched] {
                matched = prefixes[matched - 1];
            }
            if folded[index] == folded[matched] {
                matched += 1;
            }
            prefixes[index] = matched;
        }
        Ok(Self {
            text: Arc::from(text),
            folded: folded.into(),
            prefixes: prefixes.into(),
        })
    }

    /// Presentation keeps the exact typed query while whitespace-only input matches everything.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Empty matching is independent of the original typed whitespace.
    pub fn is_empty(&self) -> bool {
        self.folded.is_empty()
    }

    /// Search can narrow an admitted class but can never turn text or artwork into a player.
    pub fn includes(
        &self,
        filter: CameraSubjectFilter,
        target: &CameraTarget,
        local_actor: u64,
    ) -> bool {
        filter.includes(target, local_actor) && self.matches(target)
    }

    /// Name and captured entity type share the same case-insensitive matching semantics.
    pub fn matches(&self, target: &CameraTarget) -> bool {
        self.contains(&target.name)
            || target
                .entity_type
                .as_ref()
                .is_some_and(|kind| self.contains(kind))
    }

    /// Prefix fallback gives linear scan work and no per-subject string or lowercase allocations.
    fn contains(&self, text: &str) -> bool {
        if self.is_empty() {
            return true;
        }
        let mut matched = 0;
        for character in text.chars().flat_map(char::to_lowercase) {
            while matched > 0 && character != self.folded[matched] {
                matched = self.prefixes[matched - 1];
            }
            if character == self.folded[matched] {
                matched += 1;
            }
            if matched == self.folded.len() {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests;
