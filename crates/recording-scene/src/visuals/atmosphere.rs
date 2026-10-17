//! Frozen precipitation, lightning, sky preferences and source-camera effect layers.

use assets::weather_textures::{WeatherImage, WeatherTextures};
use recording_studio::{
    archive::ArchiveError,
    object_store::{AssetId, AssetReader, AssetWriter},
};
use render_model::{
    MAX_PRECIPITATION_LAYERS, OCCLUSION_SIDE, PARTICLE_POOL, PrecipitationLayerRecord,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Weak};

const MAX_ASSET_BYTES: usize = 128 * 1024 * 1024;
const PRECIPITATION_WORDS: usize = std::mem::size_of::<PrecipitationLayerRecord>() / 4;
const MAX_RECORDED_OVERLAYS: usize = 64;
const MAX_RECORDED_LIGHTNING: usize = 4096;

#[derive(Clone, Default)]
pub struct AtmosphereAssets {
    pub identity: [u8; 32],
    pub weather: Option<Arc<WeatherTextures>>,
    pub end_sky: Option<Arc<WeatherImage>>,
    pub pumpkin: Option<Arc<[u8]>>,
    pub spyglass: Option<Arc<[u8]>>,
    pub fire: Option<Arc<RecordedFire>>,
    pub mask_side: u32,
}

#[derive(Clone)]
pub struct RecordedFire {
    pub side: u32,
    pub frames: u32,
    pub pixels: Arc<[u8]>,
    pub timeline: Arc<[u32]>,
    pub ticks_per_frame: u32,
    pub blend: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayLayer {
    pub kind: OverlayKind,
    pub rgb: [f32; 3],
    pub alpha: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayKind {
    Flat,
    PowderSnow,
    Fire,
    Portal,
    PumpkinBlur,
    SpyglassScope,
}

#[derive(Clone)]
pub struct AtmosphereVisuals {
    pub assets: Arc<AtmosphereAssets>,
    pub precipitation: Arc<[PrecipitationLayerRecord]>,
    pub forward_offset: [f32; 3],
    pub occlusion_origin: [i32; 2],
    pub occlusion_heights: Arc<[i32]>,
    pub occlusion_generation: u64,
    /// Start.xyz, half-width, end.xyz, intensity in the native ribbon contract.
    pub lightning: Arc<[[f32; 8]]>,
    pub overlays: Arc<[OverlayLayer]>,
    pub overlay_clock: f32,
    pub fire_projection: [f32; 2],
    pub portal_from_clip: [[f32; 4]; 4],
    pub fog_weather_level: f32,
    pub current_rain_level: f32,
    pub clouds: bool,
    pub skybox_visible: bool,
    pub skybox_native_offset: bool,
}

impl AtmosphereVisuals {
    /// Charges original immutable raster ownership once for each queued prepared scene sample.
    pub(super) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + std::mem::size_of_val(&*self.precipitation)
            + std::mem::size_of_val(&*self.occlusion_heights)
            + std::mem::size_of_val(&*self.lightning)
            + std::mem::size_of_val(&*self.overlays)
            + assets_bytes(&self.assets)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Metadata {
    assets: AssetId,
    precipitation: Vec<[u32; PRECIPITATION_WORDS]>,
    forward_offset: [f32; 3],
    occlusion_origin: [i32; 2],
    occlusion_heights: Vec<i32>,
    occlusion_generation: u64,
    lightning: Vec<[f32; 8]>,
    overlays: Vec<OverlayLayer>,
    overlay_clock: f32,
    fire_projection: [f32; 2],
    portal_from_clip: [[f32; 4]; 4],
    fog_weather_level: f32,
    current_rain_level: f32,
    clouds: bool,
    skybox_visible: bool,
    skybox_native_offset: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetBinding {
    weather: Option<AssetId>,
    end_sky: Option<Image>,
    pumpkin: Option<Image>,
    spyglass: Option<Image>,
    fire: Option<Fire>,
    mask_side: u32,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Image {
    width: u32,
    height: u32,
    pixels: AssetId,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fire {
    image: Image,
    frames: u32,
    timeline: Vec<u32>,
    ticks_per_frame: u32,
    blend: bool,
}

#[derive(Default)]
pub(super) struct AssetCache(Option<(Weak<AtmosphereAssets>, AssetId)>);

/// Static weather and effect raster epochs are hashed only after an immutable source changes.
pub(super) fn store(
    store: &mut AssetWriter,
    cache: &mut AssetCache,
    frame: &AtmosphereVisuals,
) -> Result<Metadata, ArchiveError> {
    let id = match &cache.0 {
        Some((old, id))
            if old
                .upgrade()
                .is_some_and(|old| Arc::ptr_eq(&old, &frame.assets)) =>
        {
            *id
        }
        _ => {
            let id = store_assets(store, &frame.assets)?;
            cache.0 = Some((Arc::downgrade(&frame.assets), id));
            id
        }
    };
    Ok(Metadata {
        assets: id,
        precipitation: frame
            .precipitation
            .iter()
            .map(|p| bytemuck::cast(*p))
            .collect(),
        forward_offset: frame.forward_offset,
        occlusion_origin: frame.occlusion_origin,
        occlusion_heights: frame.occlusion_heights.to_vec(),
        occlusion_generation: frame.occlusion_generation,
        lightning: frame.lightning.to_vec(),
        overlays: frame.overlays.to_vec(),
        overlay_clock: frame.overlay_clock,
        fire_projection: frame.fire_projection,
        portal_from_clip: frame.portal_from_clip,
        fog_weather_level: frame.fog_weather_level,
        current_rain_level: frame.current_rain_level,
        clouds: frame.clouds,
        skybox_visible: frame.skybox_visible,
        skybox_native_offset: frame.skybox_native_offset,
    })
}

/// Writes small carrier and mask objects using digest filenames, with no runtime filesystem paths.
fn store_assets(
    store: &mut AssetWriter,
    assets: &AtmosphereAssets,
) -> Result<AssetId, ArchiveError> {
    if assets_bytes(assets) > MAX_ASSET_BYTES {
        return Err(ArchiveError::Quota);
    }
    let weather = assets
        .weather
        .as_ref()
        .map(|textures| {
            let bytes = assets::weather_textures::encode_weather_textures(textures)
                .map_err(|_| ArchiveError::Invalid("invalid recorded weather carrier"))?;
            put_bytes(store, &bytes)
        })
        .transpose()?;
    let end_sky = assets
        .end_sky
        .as_ref()
        .map(|image| {
            Ok::<_, ArchiveError>(Image {
                width: image.width,
                height: image.height,
                pixels: put_bytes(store, &image.rgba8)?,
            })
        })
        .transpose()?;
    let mut mask = |pixels: &Arc<[u8]>| {
        Ok::<_, ArchiveError>(Image {
            width: assets.mask_side,
            height: assets.mask_side,
            pixels: store.put_shared_pixels(pixels)?,
        })
    };
    let pumpkin = assets.pumpkin.as_ref().map(&mut mask).transpose()?;
    let spyglass = assets.spyglass.as_ref().map(&mut mask).transpose()?;
    let fire = assets
        .fire
        .as_ref()
        .map(|fire| {
            Ok::<_, ArchiveError>(Fire {
                image: Image {
                    width: fire.side,
                    height: fire.side,
                    pixels: store.put_shared_pixels(&fire.pixels)?,
                },
                frames: fire.frames,
                timeline: fire.timeline.to_vec(),
                ticks_per_frame: fire.ticks_per_frame,
                blend: fire.blend,
            })
        })
        .transpose()?;
    let binding = AssetBinding {
        weather,
        end_sky,
        pumpkin,
        spyglass,
        fire,
        mask_side: assets.mask_side,
    };
    store.put(1024 * 1024, |out| {
        serde_json::to_writer(out, &binding)?;
        Ok(())
    })
}

#[derive(Default)]
pub(super) struct Loader(Option<(AssetId, Arc<AtmosphereAssets>)>);

/// Every shader input and fixed weather allocation is admitted before a replay resource is exposed.
pub(super) fn load(
    loader: &mut Loader,
    store: &AssetReader,
    m: Metadata,
    cancelled: &impl Fn() -> bool,
) -> Result<AtmosphereVisuals, ArchiveError> {
    if m.precipitation.len() > MAX_PRECIPITATION_LAYERS
        || m.lightning.len() > MAX_RECORDED_LIGHTNING
        || m.overlays.len() > MAX_RECORDED_OVERLAYS
        || m.occlusion_heights.len() != 2 * (OCCLUSION_SIDE * OCCLUSION_SIDE) as usize
        || !m
            .forward_offset
            .iter()
            .chain(&m.fire_projection)
            .chain(m.portal_from_clip.iter().flatten())
            .chain([
                &m.overlay_clock,
                &m.fog_weather_level,
                &m.current_rain_level,
            ])
            .all(|v| v.is_finite())
        || m.lightning.iter().flatten().any(|v| !v.is_finite())
        || m.lightning.iter().any(|v| v[3] < 0.0 || v[7] < 0.0)
        || m.overlays.iter().any(|layer| {
            !layer.alpha.is_finite()
                || !layer.rgb.iter().all(|v| v.is_finite())
                || layer.alpha < 0.0
        })
    {
        return Err(ArchiveError::Invalid(
            "invalid recorded weather or camera effects",
        ));
    }
    let precipitation = m
        .precipitation
        .into_iter()
        .map(|words| {
            let p: PrecipitationLayerRecord = bytemuck::cast(words);
            if p.base_offset
                .iter()
                .chain(&p.velocity)
                .chain(&p.uv_rect)
                .chain(&p.dimensions)
                .any(|v| !v.is_finite())
                || p.kind > 1
                || p.particle_count > PARTICLE_POOL
                || p.first_particle >= PARTICLE_POOL
            {
                return Err(ArchiveError::Invalid(
                    "invalid recorded precipitation layer",
                ));
            }
            Ok(p)
        })
        .collect::<Result<Vec<_>, ArchiveError>>()?;
    let assets = match &loader.0 {
        Some((id, assets)) if *id == m.assets => Arc::clone(assets),
        _ => {
            let assets = Arc::new(load_assets(store, m.assets, cancelled)?);
            loader.0 = Some((m.assets, Arc::clone(&assets)));
            assets
        }
    };
    if cancelled() {
        return Err(ArchiveError::Cancelled);
    }
    Ok(AtmosphereVisuals {
        assets,
        precipitation: precipitation.into(),
        forward_offset: m.forward_offset,
        occlusion_origin: m.occlusion_origin,
        occlusion_heights: m.occlusion_heights.into(),
        occlusion_generation: m.occlusion_generation,
        lightning: m.lightning.into(),
        overlays: m.overlays.into(),
        overlay_clock: m.overlay_clock,
        fire_projection: m.fire_projection,
        portal_from_clip: m.portal_from_clip,
        fog_weather_level: m.fog_weather_level,
        current_rain_level: m.current_rain_level,
        clouds: m.clouds,
        skybox_visible: m.skybox_visible,
        skybox_native_offset: m.skybox_native_offset,
    })
}

/// Reconstructs carrier overrides and validates every raster before native GPU allocation.
fn load_assets(
    store: &AssetReader,
    id: AssetId,
    cancelled: &impl Fn() -> bool,
) -> Result<AtmosphereAssets, ArchiveError> {
    let binding: AssetBinding = store.read_cancellable(id, 1024 * 1024, cancelled, |input| {
        Ok(serde_json::from_reader(input)?)
    })?;
    let mut budget = MAX_ASSET_BYTES;
    let weather = binding
        .weather
        .map(|id| {
            let bytes = read_pixels(store, id, None, &mut budget, cancelled)?;
            assets::weather_textures::decode_weather_textures(&bytes)
                .map(|(textures, _)| Arc::new(textures))
                .map_err(|_| ArchiveError::Invalid("invalid recorded weather carrier"))
        })
        .transpose()?;
    let end_sky = binding
        .end_sky
        .map(|image| {
            let bytes = image_bytes(image.width, image.height, 1)?;
            Ok::<_, ArchiveError>(Arc::new(WeatherImage {
                width: image.width,
                height: image.height,
                rgba8: read_pixels(store, image.pixels, Some(bytes), &mut budget, cancelled)?
                    .to_vec()
                    .into_boxed_slice(),
            }))
        })
        .transpose()?;
    let mut mask = |image: Image| {
        if image.width != binding.mask_side || image.height != binding.mask_side {
            return Err(ArchiveError::Invalid(
                "invalid recorded overlay mask extent",
            ));
        }
        read_pixels(
            store,
            image.pixels,
            Some(image_bytes(image.width, image.height, 1)?),
            &mut budget,
            cancelled,
        )
    };
    let pumpkin = binding.pumpkin.map(&mut mask).transpose()?;
    let spyglass = binding.spyglass.map(&mut mask).transpose()?;
    let fire = binding
        .fire
        .map(|fire| {
            if fire.image.width != fire.image.height
                || fire.timeline.is_empty()
                || fire.timeline.len() > 65_536
                || fire.frames == 0
                || fire.ticks_per_frame == 0
                || fire.timeline.iter().any(|&layer| layer >= fire.frames)
            {
                return Err(ArchiveError::Invalid("invalid recorded fire animation"));
            }
            let bytes = image_bytes(fire.image.width, fire.image.height, fire.frames)?;
            Ok(Arc::new(RecordedFire {
                side: fire.image.width,
                frames: fire.frames,
                pixels: read_pixels(
                    store,
                    fire.image.pixels,
                    Some(bytes),
                    &mut budget,
                    cancelled,
                )?,
                timeline: fire.timeline.into(),
                ticks_per_frame: fire.ticks_per_frame,
                blend: fire.blend,
            }))
        })
        .transpose()?;
    Ok(AtmosphereAssets {
        identity: id.0,
        weather,
        end_sky,
        pumpkin,
        spyglass,
        fire,
        mask_side: binding.mask_side,
    })
}

/// Object bounds and one cumulative budget apply before every pixel buffer allocation.
fn read_pixels(
    store: &AssetReader,
    id: AssetId,
    expected: Option<usize>,
    budget: &mut usize,
    cancelled: &impl Fn() -> bool,
) -> Result<Arc<[u8]>, ArchiveError> {
    let maximum = expected.unwrap_or(assets::weather_textures::MAX_WEATHER_TEXTURES_BYTES);
    *budget = budget.checked_sub(maximum).ok_or(ArchiveError::Quota)?;
    store.read_cancellable(id, maximum as u64, cancelled, |input| {
        let mut bytes = Vec::new();
        input
            .read_to_end(&mut bytes)
            .map_err(|_| ArchiveError::Invalid("truncated recorded atmosphere image"))?;
        if expected.is_some_and(|expected| bytes.len() != expected) {
            return Err(ArchiveError::Invalid(
                "recorded atmosphere pixel size mismatch",
            ));
        }
        Ok(bytes.into())
    })
}

/// Shader raster extents use the same device-independent ceiling as other native recorded images.
fn image_bytes(width: u32, height: u32, layers: u32) -> Result<usize, ArchiveError> {
    if width == 0
        || height == 0
        || layers == 0
        || width > assets::texture::policy::MAX_TEXTURE_SIDE
        || height > assets::texture::policy::MAX_TEXTURE_SIDE
    {
        return Err(ArchiveError::Invalid(
            "invalid recorded atmosphere image extent",
        ));
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|p| p.checked_mul(layers as usize))
        .and_then(|p| p.checked_mul(4))
        .filter(|&bytes| bytes <= MAX_ASSET_BYTES)
        .ok_or(ArchiveError::Quota)
}

/// Charges only owned raster storage, not playback cursor or source-world state.
fn assets_bytes(assets: &AtmosphereAssets) -> usize {
    assets
        .weather
        .as_ref()
        .map_or(0, |a| a.weather.rgba8.len() + a.end_sky.rgba8.len())
        + assets.end_sky.as_ref().map_or(0, |a| a.rgba8.len())
        + assets.pumpkin.as_ref().map_or(0, |p| p.len())
        + assets.spyglass.as_ref().map_or(0, |p| p.len())
        + assets
            .fire
            .as_ref()
            .map_or(0, |f| f.pixels.len() + std::mem::size_of_val(&*f.timeline))
}

/// Immutable small weather rasters use the same content-addressed store as larger texture epochs.
fn put_bytes(store: &mut AssetWriter, bytes: &[u8]) -> Result<AssetId, ArchiveError> {
    store.put(bytes.len() as u64, |output| {
        output
            .write_all(bytes)
            .map_err(|_| ArchiveError::Invalid("unwritable recorded atmosphere image"))
    })
}
