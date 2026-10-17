//! Content-addressed actor rasters with bounded, independent appearance metadata.

use std::sync::Arc;

use recording_studio::{
    archive::ArchiveError,
    object_store::{AssetId, AssetReader, AssetWriter},
};
use render_api::SkinRgba8;
use render_model::{
    ActorArtworkPages, ActorSkinResidency, ActorTexturePage, EquipmentRaster,
    PLAYER_SKIN_BUDGET_BYTES, ResidentSkin, SKIN_CLASS_SIDES, actor_skin_side,
};
use serde::{Deserialize, Serialize};

use super::io_error;

const MAX_METADATA: u64 = 16 * 1024 * 1024;
pub(super) const MAX_BYTES: usize = 512 * 1024 * 1024;
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Appearance {
    version: u32,
    identity: [u8; 32],
    entity_identity: [u8; 32],
    classes: Vec<Vec<Option<Skin>>>,
    pages: Vec<Page>,
    actor_glint: Option<Raster>,
    item_glint: Option<Raster>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Skin {
    source: AssetId,
    source_bytes: usize,
    texels: AssetId,
    admission: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raster {
    width: u16,
    height: u16,
    pixels: AssetId,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    raster: Raster,
    layers: u32,
    color_mask: bool,
    multitexture: bool,
}

pub(super) struct LoadedAppearance {
    pub skins: Arc<ActorSkinResidency>,
    pub artwork: Arc<ActorArtworkPages>,
}

/// Conservatively charges raster allocations once per appearance lease, never per pose sample.
pub(super) fn retained_bytes(
    skins: &ActorSkinResidency,
    artwork: &ActorArtworkPages,
) -> Option<usize> {
    let mut total = std::mem::size_of_val(skins).checked_add(std::mem::size_of_val(artwork))?;
    for class in &skins.classes {
        total = total.checked_add(std::mem::size_of_val(&**class))?;
        for resident in class.iter().flatten() {
            total = total
                .checked_add(resident.skin.len())?
                .checked_add(resident.texels.len())?;
        }
    }
    total = total.checked_add(std::mem::size_of_val(&*artwork.pages))?;
    for page in artwork.pages.iter() {
        total = total.checked_add(page.rgba8.len())?;
    }
    for raster in [artwork.actor_glint.as_ref(), artwork.item_glint.as_ref()]
        .into_iter()
        .flatten()
    {
        total = total.checked_add(raster.rgba8.len())?;
    }
    Some(total)
}

/// Static pixels are separate objects, so a skin change never rewrites unchanged entity pages.
pub(super) fn store(
    store: &mut AssetWriter,
    skins: &ActorSkinResidency,
    artwork: &ActorArtworkPages,
) -> Result<AssetId, ArchiveError> {
    if retained_bytes(skins, artwork).is_none_or(|bytes| bytes > MAX_BYTES) {
        return Err(ArchiveError::Quota);
    }
    let classes = skins
        .classes
        .iter()
        .map(|class| {
            class
                .iter()
                .map(|resident| {
                    resident
                        .as_ref()
                        .map(|resident| {
                            let ResidentSkin {
                                skin,
                                texels,
                                admission,
                            } = resident;
                            Ok(Skin {
                                source: pixels(store, skin.pixels())?,
                                source_bytes: skin.len(),
                                texels: pixels(store, texels)?,
                                admission: *admission,
                            })
                        })
                        .transpose()
                })
                .collect::<Result<Vec<_>, ArchiveError>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let pages = artwork
        .pages
        .iter()
        .map(|page| {
            let ActorTexturePage {
                width,
                height,
                layers,
                rgba8,
                color_mask,
                multitexture,
            } = page;
            Ok(Page {
                raster: Raster {
                    width: *width,
                    height: *height,
                    pixels: pixels(store, rgba8)?,
                },
                layers: *layers,
                color_mask: *color_mask,
                multitexture: *multitexture,
            })
        })
        .collect::<Result<Vec<_>, ArchiveError>>()?;
    let actor_glint = artwork
        .actor_glint
        .as_ref()
        .map(|raster| raster_link(store, raster))
        .transpose()?;
    let item_glint = artwork
        .item_glint
        .as_ref()
        .map(|raster| raster_link(store, raster))
        .transpose()?;
    let metadata = Appearance {
        version: VERSION,
        identity: artwork.identity,
        entity_identity: artwork.entity_identity,
        classes,
        pages,
        actor_glint,
        item_glint,
    };
    store.put(MAX_METADATA, |out| {
        serde_json::to_writer(out, &metadata)?;
        Ok(())
    })
}

/// Metadata and all referenced pixels must pass checksum and aggregate admission before publication.
pub(super) fn load(
    store: &AssetReader,
    id: AssetId,
    cancelled: &impl Fn() -> bool,
) -> Result<LoadedAppearance, ArchiveError> {
    let metadata: Appearance = store.read_cancellable(id, MAX_METADATA, cancelled, |input| {
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes).map_err(io_error)?;
        Ok(serde_json::from_slice(&bytes)?)
    })?;
    if metadata.version != VERSION
        || metadata.classes.len() != SKIN_CLASS_SIDES.len()
        || metadata.pages.len() >= render_model::MAX_ACTOR_TEXTURE_PAGES
    {
        return Err(ArchiveError::Invalid(
            "unsupported recorded actor appearance",
        ));
    }
    let mut allocated = 0_usize;
    let mut budget = MAX_BYTES;
    let mut classes = Vec::with_capacity(SKIN_CLASS_SIDES.len());
    for (class, side) in metadata.classes.into_iter().zip(SKIN_CLASS_SIDES) {
        let texel_bytes = side * side * 4;
        allocated = allocated
            .checked_add(
                class
                    .len()
                    .checked_mul(texel_bytes)
                    .ok_or(ArchiveError::Quota)?,
            )
            .filter(|bytes| *bytes <= PLAYER_SKIN_BUDGET_BYTES)
            .ok_or(ArchiveError::Quota)?;
        let mut residents = Vec::with_capacity(class.len());
        for skin in class {
            residents.push(
                skin.map(|skin| {
                    if !SKIN_CLASS_SIDES
                        .iter()
                        .any(|side| side * side * 4 == skin.source_bytes)
                    {
                        return Err(ArchiveError::Invalid("invalid recorded skin source length"));
                    }
                    let source = SkinRgba8::new(read_pixels(
                        store,
                        skin.source,
                        skin.source_bytes,
                        &mut budget,
                        cancelled,
                    )?);
                    if actor_skin_side(&source).is_none() {
                        return Err(ArchiveError::Invalid(
                            "invalid recorded skin source dimensions",
                        ));
                    }
                    Ok(ResidentSkin {
                        skin: source,
                        texels: read_pixels(
                            store,
                            skin.texels,
                            texel_bytes,
                            &mut budget,
                            cancelled,
                        )?,
                        admission: skin.admission,
                    })
                })
                .transpose()?,
            );
        }
        classes.push(Arc::<[Option<ResidentSkin>]>::from(residents));
    }
    let classes = classes
        .try_into()
        .map_err(|_| ArchiveError::Invalid("recorded skin classes changed"))?;
    let mut pages = Vec::with_capacity(metadata.pages.len());
    for page in metadata.pages {
        let count = pixel_bytes(&page.raster, page.layers)?;
        pages.push(ActorTexturePage {
            width: page.raster.width,
            height: page.raster.height,
            layers: page.layers,
            rgba8: read_pixels(store, page.raster.pixels, count, &mut budget, cancelled)?,
            color_mask: page.color_mask,
            multitexture: page.multitexture,
        });
    }
    let actor_glint = read_raster(store, metadata.actor_glint, &mut budget, cancelled)?;
    let item_glint = read_raster(store, metadata.item_glint, &mut budget, cancelled)?;
    let artwork = ActorArtworkPages::from_recorded_rasters(
        metadata.identity,
        metadata.entity_identity,
        pages.into(),
        actor_glint,
        item_glint,
    )
    .ok_or(ArchiveError::Invalid(
        "invalid recorded artwork page layout",
    ))?;
    Ok(LoadedAppearance {
        skins: Arc::new(ActorSkinResidency { classes }),
        artwork: Arc::new(artwork),
    })
}

/// Raw pixels are already framed by the typed metadata and addressed by their full SHA-256.
fn pixels(store: &mut AssetWriter, bytes: &Arc<[u8]>) -> Result<AssetId, ArchiveError> {
    if bytes.is_empty() {
        return Err(ArchiveError::Invalid("empty recorded actor raster"));
    }
    store.put_shared_pixels(bytes)
}

/// Glint rasters use the same immutable pixel objects as ordinary artwork pages.
fn raster_link(store: &mut AssetWriter, raster: &EquipmentRaster) -> Result<Raster, ArchiveError> {
    let EquipmentRaster {
        width,
        height,
        rgba8,
    } = raster;
    Ok(Raster {
        width: *width,
        height: *height,
        pixels: pixels(store, rgba8)?,
    })
}

/// Checked dimensions precede allocation and the native artwork constructor checks layer limits.
fn pixel_bytes(raster: &Raster, layers: u32) -> Result<usize, ArchiveError> {
    if raster.width == 0 || raster.height == 0 || layers == 0 {
        return Err(ArchiveError::Invalid(
            "empty recorded actor raster dimensions",
        ));
    }
    usize::from(raster.width)
        .checked_mul(usize::from(raster.height))
        .and_then(|bytes| bytes.checked_mul(layers as usize))
        .and_then(|bytes| bytes.checked_mul(4))
        .ok_or(ArchiveError::Quota)
}

/// Charges all pixels before allocation; cancellation is checked by the hash reader between IO chunks.
fn read_pixels(
    store: &AssetReader,
    id: AssetId,
    bytes: usize,
    budget: &mut usize,
    cancelled: &impl Fn() -> bool,
) -> Result<Arc<[u8]>, ArchiveError> {
    *budget = budget.checked_sub(bytes).ok_or(ArchiveError::Quota)?;
    store.read_cancellable(id, bytes as u64, cancelled, |input| {
        let mut pixels = vec![0; bytes];
        input.read_exact(&mut pixels).map_err(io_error)?;
        Ok(pixels.into())
    })
}

/// Optional shader glint textures remain independent of the ordinary page routing tables.
fn read_raster(
    store: &AssetReader,
    raster: Option<Raster>,
    budget: &mut usize,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<EquipmentRaster>, ArchiveError> {
    raster
        .map(|raster| {
            let count = pixel_bytes(&raster, 1)?;
            Ok(EquipmentRaster {
                width: raster.width,
                height: raster.height,
                rgba8: read_pixels(store, raster.pixels, count, budget, cancelled)?,
            })
        })
        .transpose()
}
