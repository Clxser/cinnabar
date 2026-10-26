//! Page pixels either own one allocation or share each layer with the raster it came from, so
//! packing a session's textures into array pages never copies the compiled pixels.
use std::{borrow::Cow, hash::Hasher, sync::Arc};

/// Immutable RGBA8 bytes of an artwork page, in layer-major order.
/// Clones share their allocations; equality and hashing compare bytes, not storage.
#[derive(Clone)]
pub struct PagePixels(Storage);

#[derive(Clone)]
enum Storage {
    /// Every layer in one allocation.
    Joined(Arc<[u8]>),
    /// One shared allocation per layer, in layer order.
    Layers(Arc<[Arc<[u8]>]>),
}

impl PagePixels {
    /// Pixels whose ordered layers each keep their source allocation; one layer is stored as
    /// that allocation itself.
    pub fn from_layers<'a>(layers: impl IntoIterator<Item = &'a Arc<[u8]>>) -> Self {
        let layers: Vec<Arc<[u8]>> = layers.into_iter().map(Arc::clone).collect();
        match <[Arc<[u8]>; 1]>::try_from(layers) {
            Ok([layer]) => Self(Storage::Joined(layer)),
            Err(layers) => Self(Storage::Layers(layers.into())),
        }
    }

    /// Total bytes across every layer.
    pub fn len(&self) -> usize {
        match &self.0 {
            Storage::Joined(pixels) => pixels.len(),
            Storage::Layers(layers) => layers.iter().map(|layer| layer.len()).sum(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The allocations holding these bytes, in order; concatenated they are the page.
    pub fn parts(&self) -> &[Arc<[u8]>] {
        match &self.0 {
            Storage::Joined(pixels) => std::slice::from_ref(pixels),
            Storage::Layers(layers) => layers,
        }
    }

    /// Whether both share the same storage, so their bytes are equal without comparing them.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Storage::Joined(left), Storage::Joined(right)) => Arc::ptr_eq(left, right),
            (Storage::Layers(left), Storage::Layers(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    /// The bytes in `start..start + len`, borrowed when one allocation holds them all.
    /// Panics when the range is outside the page, like slice indexing.
    pub fn range(&self, start: usize, len: usize) -> Cow<'_, [u8]> {
        let mut offset = 0;
        for part in self.parts() {
            if start >= offset && start + len <= offset + part.len() {
                return Cow::Borrowed(&part[start - offset..start - offset + len]);
            }
            offset += part.len();
        }
        assert!(start + len <= offset, "page range out of bounds");
        let mut joined = Vec::with_capacity(len);
        let mut offset = 0;
        for part in self.parts() {
            let (from, to) = (start.max(offset), (start + len).min(offset + part.len()));
            if from < to {
                joined.extend_from_slice(&part[from - offset..to - offset]);
            }
            offset += part.len();
        }
        Cow::Owned(joined)
    }

    /// Every byte in one slice, borrowed when the page is a single allocation.
    pub fn joined(&self) -> Cow<'_, [u8]> {
        self.range(0, self.len())
    }

    /// Weak handles to each allocation, which stay upgradable only while some owner keeps them.
    pub fn downgrade(&self) -> Vec<std::sync::Weak<[u8]>> {
        self.parts().iter().map(Arc::downgrade).collect()
    }
}

impl From<Arc<[u8]>> for PagePixels {
    fn from(pixels: Arc<[u8]>) -> Self {
        Self(Storage::Joined(pixels))
    }
}

impl From<Vec<u8>> for PagePixels {
    fn from(pixels: Vec<u8>) -> Self {
        Self(Storage::Joined(pixels.into()))
    }
}

impl<const N: usize> From<[u8; N]> for PagePixels {
    fn from(pixels: [u8; N]) -> Self {
        Self(Storage::Joined(Arc::from(pixels)))
    }
}

impl std::ops::Index<usize> for PagePixels {
    type Output = u8;

    /// The byte at `index` in layer-major order; panics past the end, like slice indexing.
    fn index(&self, mut index: usize) -> &u8 {
        for part in self.parts() {
            if index < part.len() {
                return &part[index];
            }
            index -= part.len();
        }
        panic!("page pixel index out of bounds");
    }
}

impl PartialEq for PagePixels {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other)
            || (self.len() == other.len()
                && self
                    .parts()
                    .iter()
                    .flat_map(|part| part.iter())
                    .eq(other.parts().iter().flat_map(|part| part.iter())))
    }
}
impl Eq for PagePixels {}

impl PartialEq<[u8]> for PagePixels {
    fn eq(&self, other: &[u8]) -> bool {
        self.len() == other.len() && self.joined().as_ref() == other
    }
}

impl std::hash::Hash for PagePixels {
    /// Feeds the bytes in fixed-size blocks, so equal pixels hash alike whatever their storage.
    fn hash<H: Hasher>(&self, state: &mut H) {
        const BLOCK: usize = 256;
        state.write_usize(self.len());
        let mut block = [0u8; BLOCK];
        let mut filled = 0;
        for part in self.parts() {
            let mut rest: &[u8] = part;
            while !rest.is_empty() {
                let take = (BLOCK - filled).min(rest.len());
                block[filled..filled + take].copy_from_slice(&rest[..take]);
                (filled, rest) = (filled + take, &rest[take..]);
                if filled == BLOCK {
                    state.write(&block);
                    filled = 0;
                }
            }
        }
        state.write(&block[..filled]);
    }
}

impl std::fmt::Debug for PagePixels {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PagePixels")
            .field("bytes", &self.len())
            .field("allocations", &self.parts().len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::BuildHasher;

    #[test]
    fn layered_pixels_share_sources_and_compare_by_content() {
        let layers: [Arc<[u8]>; 2] = [Arc::from([1, 2, 3, 4]), Arc::from([5, 6, 7, 8])];
        let shared = PagePixels::from_layers(&layers);
        let joined = PagePixels::from(vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(Arc::ptr_eq(&shared.parts()[1], &layers[1]));
        assert_eq!(shared, joined);
        assert!(!shared.ptr_eq(&joined));
        let state = std::collections::hash_map::RandomState::new();
        assert_eq!(state.hash_one(&shared), state.hash_one(&joined));
        assert_eq!(shared.range(4, 4).as_ref(), &[5, 6, 7, 8]);
        assert!(matches!(shared.range(4, 4), Cow::Borrowed(_)));
        assert_eq!(shared.range(2, 4).as_ref(), &[3, 4, 5, 6]);
        assert_eq!(shared.joined().as_ref(), &[1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn one_layer_keeps_its_source_allocation() {
        let layer: Arc<[u8]> = Arc::from([9; 16]);
        let pixels = PagePixels::from_layers([&layer]);
        assert!(pixels.ptr_eq(&PagePixels::from(Arc::clone(&layer))));
    }
}
