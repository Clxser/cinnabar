//! Frozen material and biome identities shared by every mesh in one asset epoch.

use std::sync::Arc;

use assets::{RuntimeAssets, seasonal_foliage::SEASONAL_FOLIAGE_COUNT};
use recording_studio::{
    archive::{ArchiveError, MAX_RECORD_BYTES},
    object_store::{AssetId, AssetReader, AssetWriter, MAX_ASSET_BYTES},
};
use serde::{Deserialize, Serialize};

use crate::binary::{self, Reader};

const TINT_MAGIC: [u8; 8] = *b"CINTNT01";
const TINT_WORDS: usize = 6 * 3 + SEASONAL_FOLIAGE_COUNT * 3 + 2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrozenTint {
    pub grass: [f32; 3],
    pub foliage: [f32; 3],
    pub birch: [f32; 3],
    pub evergreen: [f32; 3],
    pub dry_foliage: [f32; 3],
    pub seasonal_foliage: [[f32; 3]; SEASONAL_FOLIAGE_COUNT],
    pub water: [f32; 3],
    pub water_opacity: f32,
    pub flags: u32,
}

impl FrozenTint {
    /// Source and shader colour samples retain their finite linear-light component values.
    fn valid(&self) -> bool {
        let Self {
            grass,
            foliage,
            birch,
            evergreen,
            dry_foliage,
            seasonal_foliage,
            water,
            water_opacity,
            flags: _,
        } = self;
        [grass, foliage, birch, evergreen, dry_foliage, water]
            .into_iter()
            .flatten()
            .chain(seasonal_foliage.iter().flatten())
            .chain(std::iter::once(water_opacity))
            .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
    }

    /// Exhaustive field binding makes an added native colour channel require a schema update.
    fn words(&self) -> [u32; TINT_WORDS] {
        let Self {
            grass,
            foliage,
            birch,
            evergreen,
            dry_foliage,
            seasonal_foliage,
            water,
            water_opacity,
            flags,
        } = self;
        let mut words = [0; TINT_WORDS];
        let colors = [grass, foliage, birch, evergreen, dry_foliage, water]
            .into_iter()
            .flatten()
            .chain(seasonal_foliage.iter().flatten())
            .chain(std::iter::once(water_opacity));
        for (slot, value) in words.iter_mut().zip(colors) {
            *slot = value.to_bits();
        }
        words[TINT_WORDS - 1] = *flags;
        words
    }

    /// Only validated finite arrays become native shader samples.
    fn from_words(words: [u32; TINT_WORDS]) -> Option<Self> {
        let color =
            |index: usize| std::array::from_fn(|axis| f32::from_bits(words[index * 3 + axis]));
        let value = Self {
            grass: color(0),
            foliage: color(1),
            birch: color(2),
            evergreen: color(3),
            dry_foliage: color(4),
            water: color(5),
            seasonal_foliage: std::array::from_fn(|index| color(6 + index)),
            water_opacity: f32::from_bits(words[TINT_WORDS - 2]),
            flags: words[TINT_WORDS - 1],
        };
        value.valid().then_some(value)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaletteBinding {
    pub world: AssetId,
    pub tints: AssetId,
    pub tint_count: u32,
    pub tint_stream: u64,
    pub tint_revision: u64,
}

pub struct FrozenPalette {
    pub binding: PaletteBinding,
    pub assets: Arc<RuntimeAssets>,
    pub tints: Arc<[FrozenTint]>,
    world_bytes: usize,
}

impl FrozenPalette {
    /// Charges large immutable tables once when the palette is admitted, not once per scene chunk.
    pub(crate) fn new(
        binding: PaletteBinding,
        assets: Arc<RuntimeAssets>,
        tints: Arc<[FrozenTint]>,
    ) -> Self {
        let world_bytes = assets.recording_retained_bytes();
        Self {
            binding,
            assets,
            tints,
            world_bytes,
        }
    }

    pub(crate) const fn world_bytes(&self) -> usize {
        self.world_bytes
    }
}

/// Freezes the exact runtime material/model/texture tables through bounded object publication.
pub fn store_world(
    store: &mut AssetWriter,
    assets: &RuntimeAssets,
) -> Result<AssetId, ArchiveError> {
    store.put(MAX_ASSET_BYTES, |output| {
        assets
            .write_recording(output, MAX_ASSET_BYTES as usize)
            .map_err(asset_error)
    })
}

/// Loads all runtime references before the outer store verifies and admits the complete asset digest.
pub fn load_world(store: &AssetReader, id: AssetId) -> Result<Arc<RuntimeAssets>, ArchiveError> {
    load_world_cancellable(store, id, || false)
}

/// A superseded seek stops reading large frozen texture tables without waiting for the entire asset.
pub(crate) fn load_world_cancellable(
    store: &AssetReader,
    id: AssetId,
    cancelled: impl Fn() -> bool,
) -> Result<Arc<RuntimeAssets>, ArchiveError> {
    store.read_cancellable(id, MAX_ASSET_BYTES, cancelled, |input| {
        RuntimeAssets::read_recording(input, MAX_ASSET_BYTES as usize)
            .map(Arc::new)
            .map_err(asset_error)
    })
}

/// Dense tint indices remain stable; repeated chunk records reference one immutable table.
pub fn store_tints(store: &mut AssetWriter, tints: &[FrozenTint]) -> Result<AssetId, ArchiveError> {
    if tints.is_empty()
        || tints.len() > (MAX_RECORD_BYTES - 12) / (TINT_WORDS * std::mem::size_of::<u32>())
        || tints.iter().any(|tint| !tint.valid())
    {
        return Err(ArchiveError::Invalid(
            "recorded biome tint table exceeds its value or byte bounds",
        ));
    }
    store.put(MAX_RECORD_BYTES as u64, |output| {
        output
            .write_all(&TINT_MAGIC)
            .map_err(|_| ArchiveError::Invalid("cannot write recorded tint header"))?;
        binary::packed(output, tints, FrozenTint::words)
    })
}

/// The count and all float components are checked before a decoded table reaches a shader.
pub fn load_tints(store: &AssetReader, id: AssetId) -> Result<Arc<[FrozenTint]>, ArchiveError> {
    load_tints_cancellable(store, id, || false)
}

/// Dense palette loads share the same cancellation contract as their parent world seek.
pub(crate) fn load_tints_cancellable(
    store: &AssetReader,
    id: AssetId,
    cancelled: impl Fn() -> bool,
) -> Result<Arc<[FrozenTint]>, ArchiveError> {
    store.read_cancellable(id, MAX_RECORD_BYTES as u64, cancelled, |input| {
        let mut reader = Reader::new(input);
        if reader.bytes::<8>()? != TINT_MAGIC {
            return Err(ArchiveError::Invalid("unsupported recorded tint schema"));
        }
        let tints = reader.packed(FrozenTint::from_words)?;
        if tints.is_empty() {
            return Err(ArchiveError::Invalid(
                "recorded tint table has no fallback entry",
            ));
        }
        drop(reader);
        binary::finish(input)?;
        Ok(tints.into())
    })
}

/// The codec preserves the asset owner's actionable explanation, without introducing a dependency cycle.
fn asset_error(error: assets::AssetError) -> ArchiveError {
    ArchiveError::Io {
        path: "frozen-world-assets".into(),
        source: std::io::Error::other(error),
    }
}
