//! Library filtering is admitted before the visible-entry budget is applied.

use super::*;

pub const MAX_LIBRARY_SEARCH_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LibrarySort {
    #[default]
    Newest,
    Oldest,
    Name,
}

impl LibrarySort {
    /// Names describe the ordering of the actual worker result.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Newest => "Newest first",
            Self::Oldest => "Oldest first",
            Self::Name => "Name A-Z",
        }
    }

    /// The native picker cycles every admitted ordering.
    pub const fn next(self) -> Self {
        match self {
            Self::Newest => Self::Oldest,
            Self::Oldest => Self::Name,
            Self::Name => Self::Newest,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LibraryQuery {
    pub search: String,
    pub sort: LibrarySort,
}

impl LibraryQuery {
    /// A bounded query performs no filesystem work at the UI boundary.
    pub fn validate(&self) -> Result<(), StorageError> {
        if self.search.len() > MAX_LIBRARY_SEARCH_BYTES || self.search.chars().any(char::is_control)
        {
            return Err(StorageError::Project(crate::StudioError::Invalid(
                "library search must be bounded text without control characters",
            )));
        }
        Ok(())
    }
}

/// Filtering precedes truncation, so matching older entries remain reachable.
pub(super) fn finish(listing: &mut LibraryListing, query: &LibraryQuery) {
    let search = query.search.trim().to_lowercase();
    if !search.is_empty() {
        listing.entries.retain(|entry| {
            entry.name.to_lowercase().contains(&search)
                || entry
                    .path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().to_lowercase().contains(&search))
        });
    }
    match query.sort {
        LibrarySort::Name => listing
            .entries
            .sort_by_cached_key(|entry| (entry.name.to_lowercase(), entry.path.clone())),
        sort => listing.entries.sort_by(|a, b| {
            let primary = if sort == LibrarySort::Newest {
                b.modified_unix_ms.cmp(&a.modified_unix_ms)
            } else {
                a.modified_unix_ms.cmp(&b.modified_unix_ms)
            };
            primary.then_with(|| a.path.cmp(&b.path))
        }),
    }
    if listing.entries.len() > MAX_LIBRARY_ENTRIES {
        listing.truncated = true;
        listing.entries.truncate(MAX_LIBRARY_ENTRIES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(index: usize) -> LibraryEntry {
        LibraryEntry {
            path: PathBuf::from(format!("take-{index}.cinstudio")),
            name: format!("Take {index}"),
            kind: LibraryKind::Project,
            modified_unix_ms: index as u64,
            interrupted: false,
        }
    }

    #[test]
    fn search_finds_an_old_match_before_the_visible_entry_limit() {
        let mut listing = LibraryListing {
            entries: (0..MAX_LIBRARY_ENTRIES + 20).map(entry).collect(),
            ..Default::default()
        };
        listing.entries[0].name = "Older CINEMATIC".into();
        let query = LibraryQuery {
            search: "cinematic".into(),
            ..Default::default()
        };
        query.validate().unwrap();
        finish(&mut listing, &query);
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].path, entry(0).path);
        assert!(!listing.truncated);
    }

    #[test]
    fn sort_changes_the_returned_order_and_retains_the_scan_warning() {
        for (sort, first) in [
            (LibrarySort::Newest, 2),
            (LibrarySort::Oldest, 0),
            (LibrarySort::Name, 0),
        ] {
            let mut listing = LibraryListing {
                entries: vec![entry(1), entry(2), entry(0)],
                truncated: true,
                issues: Vec::new(),
            };
            finish(
                &mut listing,
                &LibraryQuery {
                    sort,
                    ..Default::default()
                },
            );
            assert_eq!(listing.entries[0].path, entry(first).path);
            assert!(listing.truncated);
        }
        assert!(
            LibraryQuery {
                search: "line\nbreak".into(),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}
