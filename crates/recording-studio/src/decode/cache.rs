//! A short byte-bounded scrub window retains immutable decoded pixels, never timeline history.

use std::{collections::VecDeque, sync::Arc};

const SCRUB_FRAMES: usize = 8;
const SCRUB_BYTES: usize = 16 * 1024 * 1024;

pub(super) struct FrameCache {
    frames: VecDeque<(u64, Arc<[u8]>)>,
    bytes: usize,
    maximum: usize,
}

impl FrameCache {
    /// Exact exports only retain their previous frame; interactive scrubs retain a short recent window.
    pub fn new(interactive: bool) -> Self {
        Self {
            frames: VecDeque::new(),
            bytes: 0,
            maximum: if interactive { SCRUB_FRAMES } else { 1 },
        }
    }

    /// A cache hit does not move a decoder's forward source cursor or consume read-ahead.
    pub fn get(&mut self, index: u64) -> Option<Arc<[u8]>> {
        let position = self.frames.iter().position(|(frame, _)| *frame == index)?;
        let entry = self.frames.remove(position).expect("cached video frame");
        let pixels = Arc::clone(&entry.1);
        self.frames.push_back(entry);
        Some(pixels)
    }

    /// One admitted large frame is allowed, matching the decoder's existing single-frame bound.
    pub fn insert(&mut self, index: u64, pixels: Arc<[u8]>) {
        if self.frames.iter().any(|(frame, _)| *frame == index) {
            return;
        }
        while !self.frames.is_empty()
            && (self.frames.len() >= self.maximum
                || self.bytes.saturating_add(pixels.len()) > SCRUB_BYTES)
        {
            self.bytes -= self.frames.pop_front().expect("cached video frame").1.len();
        }
        self.bytes += pixels.len();
        self.frames.push_back((index, pixels));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearby_scrubs_share_pixels_and_evict_by_bytes_and_recent_use() {
        let mut cache = FrameCache::new(true);
        let first: Arc<[u8]> = vec![1; SCRUB_BYTES / 2].into();
        cache.insert(1, Arc::clone(&first));
        cache.insert(2, vec![2; SCRUB_BYTES / 2].into());
        assert!(Arc::ptr_eq(&first, &cache.get(1).unwrap()));
        cache.insert(3, vec![3; SCRUB_BYTES / 2].into());
        assert!(cache.get(2).is_none());
        assert!(cache.get(1).is_some());
        assert!(cache.bytes <= SCRUB_BYTES);
        cache.insert(4, vec![4; SCRUB_BYTES + 1].into());
        assert_eq!(cache.frames.len(), 1);
        assert_eq!(cache.bytes, SCRUB_BYTES + 1);
        cache.insert(5, Arc::from([5]));
        assert_eq!(cache.frames.len(), 1);
    }

    #[test]
    fn tiny_frames_still_have_a_fixed_count_bound_and_exports_keep_one() {
        let mut cache = FrameCache::new(true);
        for index in 0..100 {
            cache.insert(index, Arc::from([index as u8]));
        }
        assert_eq!(cache.frames.len(), SCRUB_FRAMES);
        let mut cache = FrameCache::new(false);
        cache.insert(0, Arc::from([0]));
        cache.insert(1, Arc::from([1]));
        assert!(cache.get(0).is_none());
        assert!(cache.get(1).is_some());
    }
}
