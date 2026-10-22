//! Media paths and captured source bindings always relocate through the same project base.

use std::path::{Path, PathBuf};

use crate::Project;

/// Relocates every independent path value; a stale saved binding is never replaced with a current source path.
pub(super) fn relocate(project: &mut Project, source_base: &Path, destination: Option<&Path>) {
    let paths = project
        .sources
        .iter_mut()
        .map(|source| &mut source.path)
        .chain(
            project
                .camera_views
                .iter_mut()
                .map(|view| &mut view.source_identity.path),
        );
    for path in paths {
        let absolute = if path.is_absolute() {
            path.clone()
        } else {
            source_base.join(&*path)
        };
        *path = destination
            .and_then(|parent| absolute.strip_prefix(parent).ok())
            .map_or_else(|| absolute.clone(), PathBuf::from);
    }
}

#[cfg(test)]
mod tests;
