//! Optional prepared particle and block-model draws, preserving their original view coverage.

use std::{
    collections::BTreeMap,
    io::{BufReader, Write},
    path::Path,
    sync::{Arc, Weak},
};

use flate2::{Compression, bufread::ZlibDecoder, write::ZlibEncoder};
use particles::replay::{PREPARED_PARTICLE_WORDS, PreparedParticle};
use particles::{AtlasPatch, MAX_LIVE_PARTICLES, ParticleInstance};
use recording_studio::{
    MediaTime,
    archive::{
        ArchiveError, RecordKind, RecordingCursor, RecordingInfo, RecordingReader, ReplayRecord,
    },
    object_store::{AssetId, AssetReader, AssetWriter},
};
use render_model::{
    BLOCK_ENTITY_VERTEX_WORDS, BlockEntityAtlasImage, BlockEntityFrame, BlockEntityVertex,
    MAX_BLOCK_ENTITY_VERTICES,
};
use serde::{Deserialize, Serialize};

use crate::binary::{self, Reader};
mod capture;
pub use capture::VisualCapture;
mod hand;
pub use hand::PreparedHand;
pub mod atmosphere;
mod hud;
mod items;
mod nametags;
mod retention;
pub use items::{ItemCoverage, PreparedItems};

pub const VISUAL_TRACK_SCHEMA: &str = "cinnabar.prepared-visuals/5";
const ITEM_TRACK_SCHEMA: &str = "cinnabar.prepared-visuals/4";
const PARTICLE_TRACK_SCHEMA: &str = "cinnabar.prepared-visuals/3";
const HUD_TRACK_SCHEMA: &str = "cinnabar.prepared-visuals/2";
const LEGACY_TRACK_SCHEMA: &str = "cinnabar.prepared-visuals/1";
pub const VISUAL_DIRECTORY: &str = "visuals.cinreplay";
const MAGIC: [u8; 8] = *b"CINVFR05";
const ITEM_MAGIC: [u8; 8] = *b"CINVFR04";
const PARTICLE_MAGIC: [u8; 8] = *b"CINVFR03";
const HUD_MAGIC: [u8; 8] = *b"CINVFR02";
const LEGACY_MAGIC: [u8; 8] = *b"CINVFR01";
const MAX_PIXELS: usize = 128 * 1024 * 1024;
const MAX_METADATA: usize = 1024 * 1024;
const MAX_RAW: usize = 64
    + MAX_METADATA
    + recording_studio::archive::MAX_RECORD_BYTES
    + render_model::MAX_UI_DRAW_BYTES
    + nametags::MAX_DRAW_BYTES
    + 64
    + 5 * MAX_BLOCK_ENTITY_VERTICES * std::mem::size_of::<BlockEntityVertex>()
    + MAX_LIVE_PARTICLES * std::mem::size_of::<ParticleInstance>();
const PARTICLE_WORDS: usize = std::mem::size_of::<ParticleInstance>() / 4;
const MAX_RECEIVED_PARTICLE_BYTES: usize =
    MAX_LIVE_PARTICLES * std::mem::size_of::<PreparedParticle>() + 4;

/// Source-view draws remain exact; newer received samples carry native billboard inputs for edited views.
#[derive(Clone, Default)]
pub struct PreparedParticles {
    pub base: Option<Arc<[u8]>>,
    pub atlas_side: u32,
    pub patch_seq: u64,
    pub patches: Arc<[AtlasPatch]>,
    pub opaque: Arc<[ParticleInstance]>,
    pub blend: Arc<[ParticleInstance]>,
    pub add: Arc<[ParticleInstance]>,
    pub received: Option<Arc<[PreparedParticle]>>,
}

#[derive(Clone, Default)]
pub struct FrozenVisuals {
    /// None identifies an unavailable legacy channel; Some(empty) is an admitted frame with no tags.
    pub nametags: Option<Arc<render_model::NametagScene>>,
    pub particles: PreparedParticles,
    pub blocks: BlockEntityFrame,
    pub block_coverage: Option<BlockCoverage>,
    pub items: Option<PreparedItems>,
    pub hand_enabled: bool,
    pub hand: Option<PreparedHand>,
    pub hud_enabled: bool,
    pub hud: Option<Arc<render_model::PreparedUiFrame>>,
    pub atmosphere: Option<atmosphere::AtmosphereVisuals>,
}

/// Received-world preparation reports deferred source data and native capacity omissions explicitly.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockCoverage {
    pub source_seconds: f64,
    pub artwork_available: bool,
    pub received_columns: u32,
    pub current_columns: u32,
    pub pending_columns: u32,
    pub submission_limit: bool,
    pub vertex_limit: bool,
    pub cache_limit: bool,
}

impl FrozenVisuals {
    /// Total referenced payload for one sample; capture retention deduplicates shared queued allocations.
    pub fn retained_bytes(&self) -> usize {
        let p = &self.particles;
        let b = &self.blocks;
        std::mem::size_of::<Self>()
            + p.base.as_ref().map_or(0, |p| p.len())
            + std::mem::size_of_val(&*p.patches)
            + p.received
                .as_ref()
                .map_or(0, |p| std::mem::size_of_val(&**p))
            + p.patches.iter().map(|p| p.rgba8.len()).sum::<usize>()
            + [p.opaque.len(), p.blend.len(), p.add.len()]
                .into_iter()
                .sum::<usize>()
                * std::mem::size_of::<ParticleInstance>()
            + b.atlas.as_ref().map_or(0, |a| a.static_rgba8.len())
            + b.dynamic_rgba8.len()
            + [
                b.solid.len(),
                b.overlay.len(),
                b.crack.len(),
                b.portal.len(),
                b.additive.len(),
            ]
            .into_iter()
            .sum::<usize>()
                * std::mem::size_of::<BlockEntityVertex>()
            + self.hand.as_ref().map_or(0, PreparedHand::retained_bytes)
            + self.items.as_ref().map_or(0, PreparedItems::retained_bytes)
            + self.hud.as_deref().map_or(0, hud::retained_bytes)
            + self.nametags.as_deref().map_or(0, nametags::retained_bytes)
            + self
                .atmosphere
                .as_ref()
                .map_or(0, atmosphere::AtmosphereVisuals::retained_bytes)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    #[serde(default)]
    nametags: Option<nametags::Metadata>,
    particle_base: Option<AssetId>,
    particle_side: u32,
    patch_seq: u64,
    patches: Vec<Patch>,
    block_atlas: Option<Atlas>,
    dynamic: Option<AssetId>,
    revision: u64,
    dynamic_revision: u64,
    portal_star_rect: [f32; 4],
    portal_time_seconds: f32,
    hand_enabled: bool,
    hand: Option<hand::Metadata>,
    #[serde(default)]
    hud_enabled: bool,
    #[serde(default)]
    hud: Option<hud::Metadata>,
    #[serde(default)]
    atmosphere: Option<atmosphere::Metadata>,
    #[serde(default)]
    received_particles: bool,
    #[serde(default)]
    block_coverage: Option<BlockCoverage>,
    #[serde(default)]
    items: Option<items::Metadata>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    seq: u64,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    pixels: AssetId,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Atlas {
    identity: [u8; 32],
    size: [u32; 2],
    static_height: u32,
    pixels: AssetId,
}

/// Hashes immutable atlas allocations only on the existing journal worker.
fn store_frame(
    output: &mut dyn Write,
    store: &mut AssetWriter,
    hand_cache: &mut hand::GeometryCache,
    atmosphere_cache: &mut atmosphere::AssetCache,
    item_cache: &mut items::Cache,
    nametag_cache: &mut nametags::Cache,
    frame: &FrozenVisuals,
) -> Result<(), ArchiveError> {
    let p = &frame.particles;
    let b = &frame.blocks;
    let (nametags, nametag_bytes) = frame
        .nametags
        .as_deref()
        .map(|scene| nametags::store(store, nametag_cache, scene))
        .transpose()?
        .map_or((None, Vec::new()), |(metadata, bytes)| {
            (Some(metadata), bytes)
        });
    let (items, item_bytes) = frame
        .items
        .as_ref()
        .map(|frame| items::store(store, item_cache, frame))
        .transpose()?
        .map_or((None, Vec::new()), |(metadata, bytes)| {
            (Some(metadata), bytes)
        });
    let (hand, hand_bytes) = match &frame.hand {
        Some(hand) if frame.hand_enabled => {
            let (metadata, bytes) = hand::store(store, hand_cache, hand)?;
            (Some(metadata), bytes)
        }
        Some(_) => {
            return Err(ArchiveError::Invalid(
                "hand data was not enabled for this recording",
            ));
        }
        None => (None, Vec::new()),
    };
    let (hud, hud_bytes) = match &frame.hud {
        Some(hud) if frame.hud_enabled => {
            let (metadata, bytes) = hud::store(store, hud)?;
            (Some(metadata), bytes)
        }
        Some(_) => {
            return Err(ArchiveError::Invalid(
                "HUD data was not enabled for this recording",
            ));
        }
        None => (None, Vec::new()),
    };
    let metadata = Metadata {
        nametags,
        particle_base: p
            .base
            .as_ref()
            .map(|p| store.put_shared_pixels(p))
            .transpose()?,
        particle_side: p.atlas_side,
        received_particles: p.received.is_some(),
        block_coverage: frame.block_coverage,
        items,
        patch_seq: p.patch_seq,
        patches: p
            .patches
            .iter()
            .map(|p| {
                Ok(Patch {
                    seq: p.seq,
                    x: p.x,
                    y: p.y,
                    width: p.width,
                    height: p.height,
                    pixels: store.put_shared_pixels(&p.rgba8)?,
                })
            })
            .collect::<Result<_, ArchiveError>>()?,
        block_atlas: b
            .atlas
            .as_ref()
            .map(|a| {
                Ok::<_, ArchiveError>(Atlas {
                    identity: a.identity,
                    size: a.size,
                    static_height: a.static_height,
                    pixels: store.put_shared_pixels(&a.static_rgba8)?,
                })
            })
            .transpose()?,
        dynamic: (!b.dynamic_rgba8.is_empty())
            .then(|| store.put_shared_pixels(&b.dynamic_rgba8))
            .transpose()?,
        revision: b.revision,
        dynamic_revision: b.dynamic_revision,
        portal_star_rect: b.portal_star_rect,
        portal_time_seconds: b.portal_time_seconds,
        hand_enabled: frame.hand_enabled,
        hand,
        hud_enabled: frame.hud_enabled,
        hud,
        atmosphere: frame
            .atmosphere
            .as_ref()
            .map(|frame| atmosphere::store(store, atmosphere_cache, frame))
            .transpose()?,
    };
    let metadata = serde_json::to_vec(&metadata)?;
    if metadata.len() > MAX_METADATA {
        return Err(ArchiveError::Quota);
    }
    let mut out = ZlibEncoder::new(output, Compression::fast());
    out.write_all(&MAGIC).map_err(io_error)?;
    binary::blob(&mut out, &metadata)?;
    for list in [&p.opaque, &p.blend, &p.add] {
        binary::packed(&mut out, list, |v| {
            bytemuck::cast::<ParticleInstance, [u32; PARTICLE_WORDS]>(*v)
        })?;
    }
    for list in [&b.solid, &b.overlay, &b.crack, &b.portal, &b.additive] {
        binary::packed(&mut out, list, |v| {
            bytemuck::cast::<BlockEntityVertex, [u32; BLOCK_ENTITY_VERTEX_WORDS]>(*v)
        })?;
    }
    binary::blob(&mut out, &hand_bytes)?;
    binary::blob(&mut out, &hud_bytes)?;
    binary::packed(&mut out, p.received.as_deref().unwrap_or(&[]), |p| {
        p.words()
    })?;
    binary::blob(&mut out, &item_bytes)?;
    binary::blob(&mut out, &nametag_bytes)?;
    out.finish().map_err(io_error)?;
    Ok(())
}

pub struct VisualRecording {
    cursor: RecordingCursor,
    store: AssetReader,
    pixels: BTreeMap<AssetId, Weak<[u8]>>,
    pending: Vec<u8>,
    current: Option<FrozenVisuals>,
    hand: hand::Loader,
    hud: hud::Loader,
    atmosphere: atmosphere::Loader,
    items: items::Loader,
    nametags: nametags::Loader,
}

impl VisualRecording {
    /// Missing tracks identify older takes; present but malformed tracks still fail admission.
    pub fn open_optional(parent: &Path) -> Result<Option<Self>, ArchiveError> {
        let root = parent.join(VISUAL_DIRECTORY);
        match std::fs::symlink_metadata(&root) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(ArchiveError::Io {
                    path: root,
                    source: e,
                });
            }
            Ok(m) if !m.is_dir() => {
                return Err(ArchiveError::Invalid("visual track is not a directory"));
            }
            Ok(_) => {}
        }
        let reader = RecordingReader::open(&root)?;
        if ![
            VISUAL_TRACK_SCHEMA,
            ITEM_TRACK_SCHEMA,
            PARTICLE_TRACK_SCHEMA,
            HUD_TRACK_SCHEMA,
            LEGACY_TRACK_SCHEMA,
        ]
        .contains(&reader.info().scene_schema.as_str())
        {
            return Err(ArchiveError::Invalid("unsupported prepared visual schema"));
        }
        Ok(Some(Self {
            cursor: reader.into_cursor(),
            store: AssetReader::new(&root),
            pixels: BTreeMap::new(),
            pending: Vec::new(),
            current: None,
            hand: hand::Loader::default(),
            hud: hud::Loader::default(),
            atmosphere: atmosphere::Loader::default(),
            items: items::Loader::default(),
            nametags: nametags::Loader::default(),
        }))
    }

    /// Seeks all prepared draws to the same admitted source timestamp as terrain and actors.
    pub fn restore(
        &mut self,
        at: MediaTime,
        cancelled: &impl Fn() -> bool,
    ) -> Result<FrozenVisuals, ArchiveError> {
        self.update(at, true, cancelled)
    }

    /// Decodes only the latest crossed sample and retains unchanged raster allocations.
    pub fn advance_to(
        &mut self,
        at: MediaTime,
        cancelled: &impl Fn() -> bool,
    ) -> Result<FrozenVisuals, ArchiveError> {
        self.update(at, false, cancelled)
    }

    /// Invalidates cursor progress together with the scene coordinator's remaining tracks.
    pub fn invalidate(&mut self) {
        self.cursor.invalidate();
    }

    pub fn duration(&self) -> MediaTime {
        self.cursor.reader().duration()
    }
    pub fn info(&self) -> &RecordingInfo {
        self.cursor.reader().info()
    }

    /// A failed or obsolete decode never becomes the base for a subsequent forward advance.
    fn update(
        &mut self,
        at: MediaTime,
        seek: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<FrozenVisuals, ArchiveError> {
        let mut found = false;
        let pending = &mut self.pending;
        let consume = |record: ReplayRecord<'_>| match record.kind {
            RecordKind::CheckpointStart => {
                found = false;
                pending.clear();
                Ok(())
            }
            RecordKind::CheckpointEnd => Ok(()),
            RecordKind::CheckpointPart | RecordKind::Scene => {
                pending.clear();
                pending.extend_from_slice(record.payload);
                found = true;
                Ok(())
            }
            _ => Err(ArchiveError::Invalid("unexpected prepared visual record")),
        };
        let progress = if seek {
            self.cursor.seek(at, cancelled, consume)
        } else {
            self.cursor.advance_to(at, cancelled, consume)
        };
        let result = progress.and_then(|()| {
            if found {
                self.load(cancelled).map(Some)
            } else if seek {
                Err(ArchiveError::Invalid("visual track has no baseline"))
            } else {
                Ok(None)
            }
        });
        match result {
            Ok(Some(frame)) => self.current = Some(frame),
            Ok(None) => {}
            Err(e) => {
                self.invalidate();
                return Err(e);
            }
        }
        self.current
            .clone()
            .ok_or(ArchiveError::Invalid("visual track has no baseline"))
    }

    /// Validates vertex values, texture dimensions and all pixel references before renderer publication.
    fn load(&mut self, cancelled: &impl Fn() -> bool) -> Result<FrozenVisuals, ArchiveError> {
        let mut decoded = ZlibDecoder::new(BufReader::new(&self.pending[..]));
        let mut r = Reader::with_limit(
            &mut decoded,
            MAX_RAW + MAX_RECEIVED_PARTICLE_BYTES + recording_studio::archive::MAX_RECORD_BYTES,
        );
        let magic = r.bytes::<8>()?;
        if magic != MAGIC
            && magic != ITEM_MAGIC
            && magic != PARTICLE_MAGIC
            && magic != HUD_MAGIC
            && magic != LEGACY_MAGIC
        {
            return Err(ArchiveError::Invalid("unsupported visual frame"));
        }
        let m: Metadata = serde_json::from_slice(&r.blob(MAX_METADATA)?)?;
        let mut particles = Vec::new();
        let mut remaining = MAX_LIVE_PARTICLES;
        for _ in 0..3 {
            let list = r.packed_limit(remaining, |w: [u32; PARTICLE_WORDS]| {
                w.iter()
                    .all(|w| f32::from_bits(*w).is_finite())
                    .then(|| bytemuck::cast::<_, ParticleInstance>(w))
            })?;
            remaining -= list.len();
            particles.push(Arc::from(list));
        }
        let mut blocks = Vec::new();
        for _ in 0..5 {
            if cancelled() {
                return Err(ArchiveError::Cancelled);
            }
            let list = r.packed_limit(
                MAX_BLOCK_ENTITY_VERTICES,
                |w: [u32; BLOCK_ENTITY_VERTEX_WORDS]| {
                    let vertex = bytemuck::cast::<_, BlockEntityVertex>(w);
                    vertex
                        .position
                        .iter()
                        .chain(&vertex.uv)
                        .chain(&vertex.color)
                        .chain(&vertex.normal)
                        .all(|v| v.is_finite())
                        .then_some(vertex)
                },
            )?;
            if list.len() % 3 != 0 {
                return Err(ArchiveError::Invalid("incomplete block model triangle"));
            }
            blocks.push(Arc::from(list));
        }
        let hand_bytes = r.blob(recording_studio::archive::MAX_RECORD_BYTES)?;
        if m.hand.is_some() != !hand_bytes.is_empty() || (m.hand.is_some() && !m.hand_enabled) {
            return Err(ArchiveError::Invalid("inconsistent recorded hand coverage"));
        }
        let hud_bytes = if magic == MAGIC
            || magic == ITEM_MAGIC
            || magic == PARTICLE_MAGIC
            || magic == HUD_MAGIC
        {
            r.blob(render_model::MAX_UI_DRAW_BYTES + 64)?
        } else {
            Vec::new()
        };
        if m.hud.is_some() != !hud_bytes.is_empty() || (m.hud.is_some() && !m.hud_enabled) {
            return Err(ArchiveError::Invalid("inconsistent recorded HUD coverage"));
        }
        let received: Option<Arc<[PreparedParticle]>> =
            if magic == MAGIC || magic == ITEM_MAGIC || magic == PARTICLE_MAGIC {
                let samples = r.packed_limit(
                    MAX_LIVE_PARTICLES,
                    |words: [u32; PREPARED_PARTICLE_WORDS]| PreparedParticle::from_words(words),
                )?;
                if !m.received_particles && !samples.is_empty() {
                    return Err(ArchiveError::Invalid(
                        "unexpected received particle samples",
                    ));
                }
                m.received_particles.then(|| Arc::from(samples))
            } else {
                None
            };
        let item_bytes = if magic == MAGIC || magic == ITEM_MAGIC {
            r.blob(recording_studio::archive::MAX_RECORD_BYTES)?
        } else {
            Vec::new()
        };
        if m.items.is_some() != !item_bytes.is_empty() {
            return Err(ArchiveError::Invalid("inconsistent recorded item coverage"));
        }
        let nametag_bytes = if magic == MAGIC {
            r.blob(nametags::MAX_DRAW_BYTES)?
        } else {
            Vec::new()
        };
        if m.nametags.is_some() != !nametag_bytes.is_empty() {
            return Err(ArchiveError::Invalid(
                "inconsistent recorded world nametag coverage",
            ));
        }
        drop(r);
        binary::finish(&mut decoded)?;
        binary::finish(&mut decoded.into_inner())?;
        if !m.portal_time_seconds.is_finite() || !m.portal_star_rect.iter().all(|v| v.is_finite()) {
            return Err(ArchiveError::Invalid("invalid recorded portal parameters"));
        }
        let mut pixel_budget = MAX_PIXELS;
        let base = m
            .particle_base
            .map(|id| {
                let size = pixel_bytes(m.particle_side, m.particle_side)?;
                self.pixels(id, size, &mut pixel_budget, cancelled)
            })
            .transpose()?;
        if base.is_none()
            && (received.as_ref().is_some_and(|samples| !samples.is_empty())
                || particles
                    .iter()
                    .any(|list: &Arc<[ParticleInstance]>| !list.is_empty()))
        {
            return Err(ArchiveError::Invalid("recorded particles have no atlas"));
        }
        if m.patches.len() > MAX_PIXELS / 4 {
            return Err(ArchiveError::Quota);
        }
        let patches = m
            .patches
            .into_iter()
            .map(|p| {
                if p.x
                    .checked_add(p.width)
                    .is_none_or(|end| end > m.particle_side)
                    || p.y
                        .checked_add(p.height)
                        .is_none_or(|end| end > m.particle_side)
                    || p.seq > m.patch_seq
                    || base.is_none()
                {
                    return Err(ArchiveError::Invalid(
                        "recorded particle patch exceeds its atlas",
                    ));
                }
                Ok(AtlasPatch {
                    seq: p.seq,
                    x: p.x,
                    y: p.y,
                    width: p.width,
                    height: p.height,
                    rgba8: self.pixels(
                        p.pixels,
                        pixel_bytes(p.width, p.height)?,
                        &mut pixel_budget,
                        cancelled,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, ArchiveError>>()?;
        let atlas = m
            .block_atlas
            .map(|a| {
                if a.static_height > a.size[1] {
                    return Err(ArchiveError::Invalid("invalid block atlas height"));
                }
                pixel_bytes(a.size[0], a.size[1])?;
                Ok(Arc::new(BlockEntityAtlasImage {
                    identity: a.identity,
                    size: a.size,
                    static_height: a.static_height,
                    static_rgba8: self.pixels(
                        a.pixels,
                        pixel_bytes(a.size[0], a.static_height)?,
                        &mut pixel_budget,
                        cancelled,
                    )?,
                }))
            })
            .transpose()?;
        let dynamic = match m.dynamic {
            Some(id) => {
                let a = atlas
                    .as_ref()
                    .ok_or(ArchiveError::Invalid("dynamic blocks have no atlas"))?;
                self.pixels(
                    id,
                    pixel_bytes(a.size[0], a.size[1] - a.static_height)?,
                    &mut pixel_budget,
                    cancelled,
                )?
            }
            None => Arc::from([]),
        };
        let [opaque, blend, add]: [Arc<[ParticleInstance]>; 3] =
            particles.try_into().map_err(|_| ArchiveError::Quota)?;
        let [solid, overlay, crack, portal, additive]: [Arc<[BlockEntityVertex]>; 5] =
            blocks.try_into().map_err(|_| ArchiveError::Quota)?;
        if atlas.is_none()
            && [&solid, &overlay, &crack, &portal]
                .iter()
                .any(|list| !list.is_empty())
        {
            return Err(ArchiveError::Invalid("recorded block models have no atlas"));
        }
        let hand = m
            .hand
            .map(|metadata| {
                let mut loader = std::mem::take(&mut self.hand);
                let store = self.store.clone();
                let result =
                    loader.load(&store, metadata, &hand_bytes, cancelled, |id, expected| {
                        self.pixels(id, expected, &mut pixel_budget, cancelled)
                    });
                self.hand = loader;
                result
            })
            .transpose()?;
        let hud = m
            .hud
            .map(|metadata| {
                let mut loader = std::mem::take(&mut self.hud);
                let mut read_budget = MAX_PIXELS;
                let result = hud::load(
                    &mut loader,
                    metadata,
                    &hud_bytes,
                    |id, expected| self.pixels(id, expected, &mut read_budget, cancelled),
                    &mut pixel_budget,
                    cancelled,
                )
                .map(Arc::new);
                self.hud = loader;
                result
            })
            .transpose()?;
        let atmosphere = m
            .atmosphere
            .map(|metadata| {
                atmosphere::load(&mut self.atmosphere, &self.store, metadata, cancelled)
            })
            .transpose()?;
        let items = m
            .items
            .map(|metadata| {
                let mut loader = std::mem::take(&mut self.items);
                let store = self.store.clone();
                let mut read_budget = MAX_PIXELS;
                let result = items::load(
                    &mut loader,
                    &store,
                    metadata,
                    &item_bytes,
                    |id, expected| self.pixels(id, expected, &mut read_budget, cancelled),
                    &mut pixel_budget,
                    cancelled,
                );
                self.items = loader;
                result
            })
            .transpose()?;
        let nametags = m
            .nametags
            .map(|metadata| {
                let mut loader = std::mem::take(&mut self.nametags);
                let mut read_budget = MAX_PIXELS;
                let result = nametags::load(
                    &mut loader,
                    metadata,
                    &nametag_bytes,
                    |id, expected| self.pixels(id, expected, &mut read_budget, cancelled),
                    &mut pixel_budget,
                    cancelled,
                );
                self.nametags = loader;
                result
            })
            .transpose()?;
        if m.block_coverage.is_some_and(|coverage| {
            !coverage.source_seconds.is_finite()
                || coverage.source_seconds < 0.0
                || coverage.current_columns > coverage.received_columns
                || coverage.pending_columns != coverage.received_columns - coverage.current_columns
        }) {
            return Err(ArchiveError::Invalid(
                "invalid received block-model coverage",
            ));
        }
        Ok(FrozenVisuals {
            nametags,
            items,
            block_coverage: m.block_coverage,
            particles: PreparedParticles {
                base,
                atlas_side: m.particle_side,
                patch_seq: m.patch_seq,
                patches: patches.into(),
                opaque,
                blend,
                add,
                received,
            },
            hand_enabled: m.hand_enabled,
            hand,
            hud_enabled: m.hud_enabled,
            hud,
            atmosphere,
            blocks: BlockEntityFrame {
                revision: m.revision,
                atlas,
                dynamic_revision: m.dynamic_revision,
                dynamic_rgba8: dynamic,
                solid,
                overlay,
                crack,
                portal,
                additive,
                portal_star_rect: m.portal_star_rect,
                portal_time_seconds: m.portal_time_seconds,
            },
        })
    }

    /// Pixel leases use weak identity caches and one cumulative resident budget per admitted frame.
    fn pixels(
        &mut self,
        id: AssetId,
        expected: usize,
        budget: &mut usize,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Arc<[u8]>, ArchiveError> {
        *budget = budget.checked_sub(expected).ok_or(ArchiveError::Quota)?;
        if cancelled() {
            return Err(ArchiveError::Cancelled);
        }
        if let Some(pixels) = self.pixels.get(&id).and_then(Weak::upgrade) {
            if pixels.len() != expected {
                return Err(ArchiveError::Invalid("conflicting recorded pixel size"));
            }
            return Ok(pixels);
        }
        let pixels: Arc<[u8]> =
            self.store
                .read_cancellable(id, expected as u64, cancelled, |input| {
                    let mut pixels = vec![0; expected];
                    input.read_exact(&mut pixels).map_err(io_error)?;
                    Ok(pixels.into())
                })?;
        self.pixels.retain(|_, value| value.strong_count() != 0);
        self.pixels.insert(id, Arc::downgrade(&pixels));
        Ok(pixels)
    }
}

/// Texture sizes are admitted before allocation and must represent a nonempty RGBA8 raster.
fn pixel_bytes(width: u32, height: u32) -> Result<usize, ArchiveError> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|p| p.checked_mul(4))
        .filter(|&bytes| bytes > 0 && bytes <= MAX_PIXELS)
        .ok_or(ArchiveError::Quota)
}

/// Preserves archive diagnostics without coupling prepared visual data to a live path.
fn io_error(_: std::io::Error) -> ArchiveError {
    ArchiveError::Invalid("truncated or unwritable prepared visuals")
}
