//! Search results retain the exact immutable recorded frame and never identify subjects by name.

use recording_studio::{
    Id,
    camera::{CameraSubjectFilter, CameraSubjectSearch, CameraTarget, CameraTargetRef},
    preview::CameraPreview,
};
use std::sync::Arc;

pub const CAMERA_SUBJECT_ROWS: usize = 4;

#[derive(Clone)]
pub struct CameraSubjectResults {
    source: Id,
    targets: Arc<[CameraTarget]>,
    query: CameraSubjectSearch,
    filter: CameraSubjectFilter,
    local_actor: u64,
    indices: Arc<[usize]>,
}

impl CameraSubjectResults {
    /// Rebuild only for a changed source, target publication, compiled query or class filter.
    pub fn new(
        preview: &CameraPreview,
        query: &CameraSubjectSearch,
        filter: CameraSubjectFilter,
    ) -> Self {
        let indices = preview
            .targets
            .iter()
            .enumerate()
            .filter_map(|(index, target)| {
                query
                    .includes(filter, target, preview.local_actor)
                    .then_some(index)
            })
            .collect();
        Self {
            source: preview.source,
            targets: Arc::clone(&preview.targets),
            query: query.clone(),
            filter,
            local_actor: preview.local_actor,
            indices,
        }
    }

    /// Pointer identity avoids walking unchanged actor arrays during ordinary paints/publications.
    pub fn is_current(
        &self,
        preview: &CameraPreview,
        query: &CameraSubjectSearch,
        filter: CameraSubjectFilter,
    ) -> bool {
        self.source == preview.source
            && Arc::ptr_eq(&self.targets, &preview.targets)
            && self.query == *query
            && self.filter == filter
            && self.local_actor == preview.local_actor
    }

    /// Matching order stays in captured-frame order; names never become stable identities.
    pub fn target(&self, index: usize) -> Option<&CameraTarget> {
        self.targets.get(*self.indices.get(index)?)
    }

    /// The result count is cached with the indices rather than rescanning names per UI paint.
    pub fn len(&self) -> usize {
        self.indices.len()
    }

    /// Empty query results are distinct from an unavailable recorded frame.
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Captured-order indices support viewport membership without rerunning text matching per paint.
    pub fn includes_index(&self, index: usize) -> bool {
        self.indices.binary_search(&index).is_ok()
    }

    /// Every result remains reachable with bounded native control geometry.
    pub fn range(&self, first: usize) -> std::ops::Range<usize> {
        let start = first.min(self.len().saturating_sub(CAMERA_SUBJECT_ROWS));
        start..start.saturating_add(CAMERA_SUBJECT_ROWS).min(self.len())
    }
}

impl PartialEq for CameraSubjectResults {
    /// Snapshot equality is constant in target count for shared publications.
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && Arc::ptr_eq(&self.targets, &other.targets)
            && self.query == other.query
            && self.filter == other.filter
            && self.local_actor == other.local_actor
    }
}

impl std::fmt::Debug for CameraSubjectResults {
    /// Debugging UI state never dumps recorded actors or their full pose arrays.
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output
            .debug_struct("CameraSubjectResults")
            .field("source", &self.source)
            .field("matches", &self.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CameraSubjectsAction {
    Search,
    Done,
    Clear,
    Page(i8),
    Select {
        source: Id,
        epoch: [u64; 3],
        target: CameraTargetRef,
    },
}

#[cfg(test)]
mod tests;
