//! Geometry animation frames follow the lifetime of their immutable source layouts.

use std::sync::{Arc, Weak};

use json_ui::{Animator, DrawNode, FormRender, GeometryLayout, LayoutEnv, ViewState};

#[cfg(test)]
mod tests;

struct Entry {
    source: Weak<[DrawNode]>,
    root: [f64; 2],
    layout: Option<GeometryLayout>,
}

#[derive(Default)]
pub(super) struct GeometryLayouts {
    entries: Vec<Entry>,
}

impl GeometryLayouts {
    /// Samples one live immutable layout; expired layouts release their programs and caches.
    #[allow(
        clippy::too_many_arguments,
        reason = "frame inputs belong to their existing owners"
    )]
    pub(super) fn sample(
        &mut self,
        source: &FormRender,
        root: [f64; 2],
        env: &LayoutEnv,
        view: Option<&ViewState>,
        animator: &mut Animator,
        now: f64,
        clocks: Option<&std::collections::BTreeMap<String, f64>>,
        allowed: impl Fn(&str) -> bool,
    ) -> Option<Arc<FormRender>> {
        self.entries
            .retain(|entry| entry.source.strong_count() != 0);
        let source_id = Arc::downgrade(&source.nodes);
        let index = self
            .entries
            .iter()
            .position(|entry| entry.root == root && entry.source.ptr_eq(&source_id));
        let default_view = ViewState::default();
        let view = view.unwrap_or(&default_view);
        let index = index.unwrap_or_else(|| {
            self.entries.push(Entry {
                source: source_id,
                root,
                layout: GeometryLayout::new(&source.bound, root, env, view),
            });
            self.entries.len() - 1
        });
        Some(
            self.entries[index]
                .layout
                .as_mut()?
                .sample(animator, now, clocks, env, view, allowed),
        )
    }
}
