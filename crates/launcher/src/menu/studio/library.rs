//! Search and ordering apply only to the local Studio library view.

use recording_studio::storage::{LibraryQuery, LibrarySort};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LibraryAction {
    Search,
    ApplySearch,
    CancelSearch,
    ClearSearch,
    CycleSort,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LibraryWorkspace {
    pub generation: u64,
    pub search: Arc<str>,
    pub draft: Option<Arc<str>>,
    pub sort: LibrarySort,
}
impl LibraryWorkspace {
    /// Produces one bounded worker query without changing any project data.
    pub fn query(&self) -> LibraryQuery {
        LibraryQuery {
            search: self.search.to_string(),
            sort: self.sort,
        }
    }
}
