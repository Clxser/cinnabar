//! Immutable native item models, pixels and fixed-point template quads.

use super::*;
use render_model::{
    DroppedItemBlock, DroppedItemCube, DroppedItemSprite, MAX_ITEM_LAYERS, MAX_ITEM_SPRITE_SIDE,
};
use std::{io::Write, sync::Weak};

const MAGIC: [u8; 8] = *b"CINITEM1";
const MAX_GEOMETRY: usize = 32 * 1024 * 1024;
const MAX_QUADS: usize = MAX_GEOMETRY / std::mem::size_of::<assets::ModelQuad>();

#[derive(Default)]
pub(in crate::visuals) struct Cache(Option<(Weak<[DroppedItemModel]>, AssetId)>);

/// Queue admission includes every retained template and source raster.
pub(super) fn retained_bytes(models: &[DroppedItemModel]) -> usize {
    std::mem::size_of_val(models)
        + pixel_bytes(models)
        + models
            .iter()
            .map(|model| match model {
                DroppedItemModel::Block(block) => {
                    std::mem::size_of_val(&*block.quads) + std::mem::size_of_val(&*block.materials)
                }
                _ => 0,
            })
            .sum::<usize>()
}

/// Cached model admission still charges the complete source raster epoch.
pub(super) fn pixel_bytes(models: &[DroppedItemModel]) -> usize {
    models
        .iter()
        .map(|model| match model {
            DroppedItemModel::Vacant => 0,
            DroppedItemModel::Sprite(sprite) | DroppedItemModel::NativeSprite(sprite) => {
                sprite.rgba8.len()
            }
            DroppedItemModel::Cube(cube) => cube.faces.iter().map(|p| p.len()).sum(),
            DroppedItemModel::Block(block) => {
                block.materials.iter().map(|(s, _)| s.rgba8.len()).sum()
            }
        })
        .sum()
}

/// Retains one weak catalog key; unchanged source allocations are never rehashed per frame.
pub(super) fn store(
    store: &mut AssetWriter,
    cache: &mut Cache,
    models: &Arc<[DroppedItemModel]>,
) -> Result<AssetId, ArchiveError> {
    if let Some((source, id)) = &cache.0
        && source
            .upgrade()
            .is_some_and(|source| Arc::ptr_eq(&source, models))
    {
        return Ok(*id);
    }
    if models.len() >= MAX_ITEM_LAYERS || pixel_bytes(models) > super::super::MAX_PIXELS {
        return Err(ArchiveError::Quota);
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&MAGIC);
    binary::u32(&mut bytes, models.len() as u32)?;
    let mut quads = 0usize;
    let mut layers = 1usize;
    for model in models.iter() {
        match model {
            DroppedItemModel::Vacant => binary::u32(&mut bytes, 0)?,
            DroppedItemModel::Sprite(sprite) | DroppedItemModel::NativeSprite(sprite) => {
                binary::u32(
                    &mut bytes,
                    if matches!(model, DroppedItemModel::Sprite(_)) {
                        1
                    } else {
                        2
                    },
                )?;
                write_sprite(&mut bytes, store, sprite)?;
                layers += 1;
            }
            DroppedItemModel::Cube(cube) => {
                binary::u32(&mut bytes, 3)?;
                binary::u32(&mut bytes, cube.tile)?;
                for face in &cube.faces {
                    if face.len() != super::super::pixel_bytes(cube.tile, cube.tile)? {
                        return Err(ArchiveError::Invalid("invalid recorded cube raster"));
                    }
                    bytes.extend_from_slice(&store.put_shared_pixels(face)?.0);
                }
                for tint in cube.tints {
                    binary::u32(&mut bytes, tint)?;
                }
                layers += cube.faces.len();
            }
            DroppedItemModel::Block(block) => {
                binary::u32(&mut bytes, 4)?;
                binary::u32(&mut bytes, block.rotation)?;
                binary::u32(&mut bytes, block.materials.len() as u32)?;
                for (sprite, tint) in block.materials.iter() {
                    write_sprite(&mut bytes, store, sprite)?;
                    binary::u32(&mut bytes, *tint)?;
                }
                quads += block.quads.len();
                if quads > MAX_QUADS {
                    return Err(ArchiveError::Quota);
                }
                binary::packed(&mut bytes, &block.quads, quad_words)?;
                layers += block.materials.len();
            }
        }
        if layers > MAX_ITEM_LAYERS || bytes.len() > MAX_GEOMETRY {
            return Err(ArchiveError::Quota);
        }
    }
    let id = store.put(MAX_GEOMETRY as u64, |out| {
        out.write_all(&bytes).map_err(super::super::io_error)
    })?;
    cache.0 = Some((Arc::downgrade(models), id));
    Ok(id)
}

/// Source image dimensions and actual texels must agree before recording object publication.
fn write_sprite(
    out: &mut dyn Write,
    store: &mut AssetWriter,
    sprite: &DroppedItemSprite,
) -> Result<(), ArchiveError> {
    if sprite.width > MAX_ITEM_SPRITE_SIDE
        || sprite.height > MAX_ITEM_SPRITE_SIDE
        || sprite.rgba8.len() != super::super::pixel_bytes(sprite.width, sprite.height)?
    {
        return Err(ArchiveError::Invalid("invalid recorded item sprite"));
    }
    binary::u32(out, sprite.width)?;
    binary::u32(out, sprite.height)?;
    out.write_all(&store.put_shared_pixels(&sprite.rgba8)?.0)
        .map_err(super::super::io_error)
}

/// A catalog is not drawable until its complete byte stream and content digest have been checked.
pub(super) fn load(
    store: &AssetReader,
    id: AssetId,
    pixels: &mut impl FnMut(AssetId, usize) -> Result<Arc<[u8]>, ArchiveError>,
    budget: &mut usize,
    cancelled: &impl Fn() -> bool,
) -> Result<Arc<[DroppedItemModel]>, ArchiveError> {
    store.read_cancellable(id, MAX_GEOMETRY as u64, cancelled, |input| {
        let mut r = Reader::with_limit(input, MAX_GEOMETRY);
        if r.bytes::<8>()? != MAGIC {
            return Err(ArchiveError::Invalid("unsupported item catalog"));
        }
        let count = r.u32()? as usize;
        if count >= MAX_ITEM_LAYERS {
            return Err(ArchiveError::Quota);
        }
        let mut models = Vec::with_capacity(count);
        let mut layers = 1usize;
        let mut quads_left = MAX_QUADS;
        for _ in 0..count {
            if cancelled() {
                return Err(ArchiveError::Cancelled);
            }
            let model = match r.u32()? {
                0 => DroppedItemModel::Vacant,
                kind @ (1 | 2) => {
                    let sprite = read_sprite(&mut r, pixels, budget)?;
                    layers += 1;
                    if kind == 1 {
                        DroppedItemModel::Sprite(sprite)
                    } else {
                        DroppedItemModel::NativeSprite(sprite)
                    }
                }
                3 => {
                    let tile = r.u32()?;
                    if tile > MAX_ITEM_SPRITE_SIDE {
                        return Err(ArchiveError::Quota);
                    }
                    let size = super::super::pixel_bytes(tile, tile)?;
                    let mut faces = Vec::with_capacity(6);
                    for _ in 0..6 {
                        faces.push(read_pixels(&mut r, size, pixels, budget)?);
                    }
                    let mut tints = [0; 6];
                    for tint in &mut tints {
                        *tint = r.u32()?;
                    }
                    layers += faces.len();
                    DroppedItemModel::Cube(DroppedItemCube {
                        tile,
                        faces: faces.try_into().map_err(|_| ArchiveError::Quota)?,
                        tints,
                    })
                }
                4 => {
                    let rotation = r.u32()?;
                    let count = r.u32()? as usize;
                    if rotation > 3 || count == 0 || count > MAX_ITEM_LAYERS {
                        return Err(ArchiveError::Invalid("invalid item block materials"));
                    }
                    let mut materials = Vec::with_capacity(count);
                    for _ in 0..count {
                        materials.push((read_sprite(&mut r, pixels, budget)?, r.u32()?));
                    }
                    let quads = r.packed_limit(quads_left, |w: [u32; 12]| {
                        let q = quad(w);
                        q.is_well_formed(materials.len()).then_some(q)
                    })?;
                    quads_left -= quads.len();
                    layers += materials.len();
                    DroppedItemModel::Block(DroppedItemBlock {
                        materials: materials.into(),
                        quads: quads.into(),
                        rotation,
                    })
                }
                _ => return Err(ArchiveError::Invalid("unknown recorded item model")),
            };
            if layers > MAX_ITEM_LAYERS {
                return Err(ArchiveError::Quota);
            }
            models.push(model);
        }
        drop(r);
        binary::finish(input)?;
        Ok(models.into())
    })
}

/// One typed sprite charges the total frame raster budget before allocating any texels.
fn read_sprite(
    r: &mut Reader<'_>,
    pixels: &mut impl FnMut(AssetId, usize) -> Result<Arc<[u8]>, ArchiveError>,
    budget: &mut usize,
) -> Result<DroppedItemSprite, ArchiveError> {
    let width = r.u32()?;
    let height = r.u32()?;
    if width > MAX_ITEM_SPRITE_SIDE || height > MAX_ITEM_SPRITE_SIDE {
        return Err(ArchiveError::Quota);
    }
    let size = super::super::pixel_bytes(width, height)?;
    Ok(DroppedItemSprite {
        width,
        height,
        rgba8: read_pixels(r, size, pixels, budget)?,
    })
}

/// Fixed digest references cannot select paths outside the recording object store.
fn read_pixels(
    r: &mut Reader<'_>,
    size: usize,
    pixels: &mut impl FnMut(AssetId, usize) -> Result<Arc<[u8]>, ArchiveError>,
    budget: &mut usize,
) -> Result<Arc<[u8]>, ArchiveError> {
    *budget = budget.checked_sub(size).ok_or(ArchiveError::Quota)?;
    pixels(AssetId(r.bytes::<32>()?), size)
}

/// Fixed-point fields use explicit little-endian words, preserving signed positions exactly.
fn quad_words(q: &assets::ModelQuad) -> [u32; 12] {
    let positions = q.positions.as_flattened();
    let uvs = q.uvs.as_flattened();
    let mut words = [0; 12];
    for (word, pair) in words[..6].iter_mut().zip(positions.chunks_exact(2)) {
        *word = u32::from(pair[0] as u16) | (u32::from(pair[1] as u16) << 16);
    }
    for (word, pair) in words[6..10].iter_mut().zip(uvs.chunks_exact(2)) {
        *word = u32::from(pair[0]) | (u32::from(pair[1]) << 16);
    }
    words[10] = q.material;
    words[11] = q.flags;
    words
}

/// Every material reference and authored flag is checked by the owning assets contract after decode.
fn quad(w: [u32; 12]) -> assets::ModelQuad {
    let mut positions = [[0; 3]; 4];
    let mut uvs = [[0; 2]; 4];
    for (pair, word) in positions
        .as_flattened_mut()
        .chunks_exact_mut(2)
        .zip(&w[..6])
    {
        pair[0] = *word as i16;
        pair[1] = (*word >> 16) as i16;
    }
    for (pair, word) in uvs.as_flattened_mut().chunks_exact_mut(2).zip(&w[6..10]) {
        pair[0] = *word as u16;
        pair[1] = (*word >> 16) as u16;
    }
    assets::ModelQuad {
        positions,
        uvs,
        material: w[10],
        flags: w[11],
    }
}
