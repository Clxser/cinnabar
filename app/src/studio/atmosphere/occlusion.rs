//! Recording shares a typed height projection while the immutable native roof grid is unchanged.

use render_model::OcclusionGrid;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Projection {
    source: Option<Arc<OcclusionGrid>>,
    heights: Arc<[i32]>,
}

impl Projection {
    /// Source ownership prevents address reuse and forces native copy-on-write to publish new data.
    pub(super) fn project(&mut self, source: &Arc<OcclusionGrid>) -> Arc<[i32]> {
        if self
            .source
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, source))
        {
            self.heights = Arc::from(&source.heights[..]);
            self.source = Some(Arc::clone(source));
        }
        Arc::clone(&self.heights)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_native_grid_shares_one_recorded_height_buffer() {
        let source = Arc::new(OcclusionGrid::default());
        let mut projection = Projection::default();
        let first = projection.project(&source);
        for _ in 0..64 {
            assert!(Arc::ptr_eq(&first, &projection.project(&source)));
        }
        assert_eq!(first.as_ref(), source.heights.as_slice());
    }

    #[test]
    fn native_copy_on_write_does_not_modify_an_accepted_sample() {
        let mut source = Arc::new(OcclusionGrid::default());
        let mut projection = Projection::default();
        let first = projection.project(&source);
        let before = first[0];
        Arc::make_mut(&mut source).heights[0] = before.wrapping_add(1);
        let second = projection.project(&source);
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(first[0], before);
        assert_eq!(second[0], before.wrapping_add(1));
    }

    #[test]
    fn replacement_and_drop_release_previous_native_grid_owners() {
        let source = Arc::new(OcclusionGrid::default());
        let weak = Arc::downgrade(&source);
        let mut projection = Projection::default();
        let first = projection.project(&source);
        drop(source);
        assert!(weak.upgrade().is_some());
        let next = Arc::new(OcclusionGrid::default());
        let second = projection.project(&next);
        assert!(weak.upgrade().is_none());
        assert_eq!(first, second);
        let weak = Arc::downgrade(&next);
        let prepared = Arc::downgrade(&second);
        drop(next);
        drop(second);
        drop(projection);
        assert!(weak.upgrade().is_none());
        assert!(prepared.upgrade().is_none());
    }
}
