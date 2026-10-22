//! Prepared world-space tag quads and exact font atlas cells remain separate from screen HUD/chat.

use crate::binary::{self, Reader};
use recording_studio::{
    archive::ArchiveError,
    object_store::{AssetId, AssetWriter},
};
use render_model::{
    MAX_NAMETAG_ATLAS_CELLS, MAX_NAMETAG_RECORDS, NAMETAG_ATLAS_SIDE, NametagAtlasRect,
    NametagLabel, NametagRecord, NametagScene,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

const RECORD_WORDS: usize = 20;
const _: () = assert!(std::mem::size_of::<NametagRecord>() == RECORD_WORDS * 4);
const MAX_ATLAS_BYTES: usize = NAMETAG_ATLAS_SIDE as usize * NAMETAG_ATLAS_SIDE as usize * 4;
pub(super) const MAX_DRAW_BYTES: usize = 4 + MAX_NAMETAG_RECORDS * RECORD_WORDS * 4;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Metadata {
    revision: u64,
    cells: Vec<Cell>,
    #[serde(default)]
    labels: Vec<Label>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Label {
    records: [usize; 2],
    view_depth: f32,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cell {
    cell: [u32; 4],
    pixels: AssetId,
}

/// Only the current immutable atlas allocation is cached; records still describe each captured frame.
#[derive(Default)]
pub(super) struct Cache {
    atlas: Option<Arc<[NametagAtlasRect]>>,
    cells: Vec<Cell>,
}

/// A decoder retains one admitted atlas epoch, with pixels owned by its published frames.
#[derive(Default)]
pub(super) struct Loader {
    cached: Option<(Vec<Cell>, Arc<[NametagAtlasRect]>, usize)>,
}

/// Queue admission accounts for all referenced texels, even when unchanged across nearby frames.
pub(super) fn retained_bytes(scene: &NametagScene) -> usize {
    std::mem::size_of::<NametagScene>()
        + std::mem::size_of_val(&*scene.records)
        + std::mem::size_of_val(&*scene.labels)
        + std::mem::size_of_val(&*scene.atlas)
        + scene
            .atlas
            .iter()
            .map(|cell| cell.rgba8.len())
            .sum::<usize>()
}

/// Native font pixels are content addressed once per immutable allocation on the existing archive worker.
pub(super) fn store(
    store: &mut AssetWriter,
    cache: &mut Cache,
    scene: &NametagScene,
) -> Result<(Metadata, Vec<u8>), ArchiveError> {
    if scene.records.len() > MAX_NAMETAG_RECORDS
        || !scene.records.iter().all(valid_record)
        || scene.labels.len() > MAX_NAMETAG_RECORDS
        || scene.labels.iter().any(|label| {
            label.records.is_empty()
                || label.records.end > scene.records.len()
                || !label.view_depth.is_finite()
        })
    {
        return Err(invalid());
    }
    let cached = cache
        .atlas
        .as_ref()
        .is_some_and(|old| Arc::ptr_eq(old, &scene.atlas));
    if !cached {
        validate_cells(scene.atlas.iter().map(|cell| cell.cell))?;
        let known = cache
            .atlas
            .iter()
            .flat_map(|atlas| atlas.iter())
            .zip(&cache.cells)
            .map(|(cell, metadata)| {
                (
                    Arc::as_ptr(&cell.rgba8) as *const u8 as usize,
                    (&cell.rgba8, metadata.pixels),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let cells = scene
            .atlas
            .iter()
            .map(|cell| {
                if cell.rgba8.len() != cell_bytes(cell.cell)? {
                    return Err(invalid());
                }
                let key = Arc::as_ptr(&cell.rgba8) as *const u8 as usize;
                let pixels = match known
                    .get(&key)
                    .filter(|(old, _)| Arc::ptr_eq(old, &cell.rgba8))
                {
                    Some((_, id)) => *id,
                    None => store.put(cell.rgba8.len() as u64, |out| {
                        std::io::Write::write_all(out, &cell.rgba8).map_err(|_| invalid())
                    })?,
                };
                Ok(Cell {
                    cell: cell.cell,
                    pixels,
                })
            })
            .collect::<Result<Vec<_>, ArchiveError>>()?;
        cache.atlas = Some(Arc::clone(&scene.atlas));
        cache.cells = cells;
    }
    if scene.atlas.is_empty() && scene.records.iter().any(|record| record.uv[2] >= 0.0) {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    binary::packed(&mut bytes, &scene.records, |record| {
        bytemuck::cast::<NametagRecord, [u32; RECORD_WORDS]>(*record)
    })?;
    Ok((
        Metadata {
            revision: scene.atlas_revision,
            cells: cache.cells.clone(),
            labels: scene
                .labels
                .iter()
                .map(|label| Label {
                    records: [label.records.start, label.records.end],
                    view_depth: label.view_depth,
                })
                .collect(),
        },
        bytes,
    ))
}

/// Counts, native flags, atlas overlap, byte ownership and floats are admitted before GPU publication.
pub(super) fn load(
    loader: &mut Loader,
    metadata: Metadata,
    bytes: &[u8],
    mut pixels: impl FnMut(AssetId, usize) -> Result<Arc<[u8]>, ArchiveError>,
    budget: &mut usize,
    cancelled: &impl Fn() -> bool,
) -> Result<Arc<NametagScene>, ArchiveError> {
    let mut input = bytes;
    let mut reader = Reader::with_limit(&mut input, MAX_DRAW_BYTES);
    let records = reader.packed_limit(MAX_NAMETAG_RECORDS, |words: [u32; RECORD_WORDS]| {
        let record = bytemuck::cast::<_, NametagRecord>(words);
        valid_record(&record).then_some(record)
    })?;
    drop(reader);
    binary::finish(&mut input)?;
    if metadata.labels.len() > MAX_NAMETAG_RECORDS
        || metadata.labels.iter().any(|label| {
            label.records[0] >= label.records[1]
                || label.records[1] > records.len()
                || !label.view_depth.is_finite()
        })
    {
        return Err(invalid());
    }
    // Legacy takes retain their original quad order as one label, without inventing actor groups.
    let labels = if metadata.labels.is_empty() && !records.is_empty() {
        vec![NametagLabel {
            records: 0..records.len(),
            view_depth: 0.0,
        }]
    } else {
        metadata
            .labels
            .into_iter()
            .map(|label| NametagLabel {
                records: label.records[0]..label.records[1],
                view_depth: label.view_depth,
            })
            .collect()
    };
    let cached = loader
        .cached
        .as_ref()
        .filter(|(old, _, _)| old.as_slice() == metadata.cells.as_slice());
    let resident = match cached {
        Some((_, _, bytes)) => *bytes,
        None => validate_cells(metadata.cells.iter().map(|cell| cell.cell))?,
    };
    *budget = budget.checked_sub(resident).ok_or(ArchiveError::Quota)?;
    if metadata.cells.is_empty() && records.iter().any(|record| record.uv[2] >= 0.0) {
        return Err(invalid());
    }
    if cancelled() {
        return Err(ArchiveError::Cancelled);
    }
    let atlas = if let Some((_, atlas, _)) = cached {
        Arc::clone(atlas)
    } else {
        let atlas = metadata
            .cells
            .iter()
            .map(|cell| {
                if cancelled() {
                    return Err(ArchiveError::Cancelled);
                }
                let expected = cell_bytes(cell.cell)?;
                let rgba8 = pixels(cell.pixels, expected)?;
                if rgba8.len() != expected {
                    return Err(invalid());
                }
                Ok(NametagAtlasRect {
                    cell: cell.cell,
                    rgba8,
                })
            })
            .collect::<Result<Vec<_>, ArchiveError>>()?;
        let atlas: Arc<[NametagAtlasRect]> = atlas.into();
        loader.cached = Some((metadata.cells, Arc::clone(&atlas), resident));
        atlas
    };
    Ok(Arc::new(NametagScene {
        records,
        labels,
        atlas,
        atlas_revision: metadata.revision,
    }))
}

/// Closed native flags and complete float checks reject malformed quads without changing tag artwork.
fn valid_record(record: &NametagRecord) -> bool {
    let NametagRecord {
        anchor,
        text,
        rect,
        uv,
        color,
        line_lift,
        depth_tested,
        runtime_id: _,
    } = record;
    anchor
        .iter()
        .chain(rect)
        .chain(uv)
        .chain(color)
        .chain([line_lift])
        .all(|v| v.is_finite())
        && *text <= 1
        && *depth_tested <= 1
        && rect[0] <= rect[2]
        && rect[1] <= rect[3]
        && color.iter().all(|v| (0.0..=1.0).contains(v))
        && (uv[2] < 0.0
            || (uv.iter().all(|v| (0.0..=1.0).contains(v)) && uv[0] <= uv[2] && uv[1] <= uv[3]))
}

/// Valid atlas rectangles occupy disjoint texels; a sweep keeps admission bounded by cells, not pixels.
fn validate_cells(cells: impl ExactSizeIterator<Item = [u32; 4]>) -> Result<usize, ArchiveError> {
    if cells.len() > MAX_NAMETAG_ATLAS_CELLS {
        return Err(ArchiveError::Quota);
    }
    let mut events = Vec::with_capacity(cells.len() * 2);
    let mut bytes = 0usize;
    for cell @ [x, y, width, height] in cells {
        bytes = bytes
            .checked_add(cell_bytes(cell)?)
            .filter(|&bytes| bytes <= MAX_ATLAS_BYTES)
            .ok_or(ArchiveError::Quota)?;
        events.push((x, true, y, y + height));
        events.push((x + width, false, y, y + height));
    }
    events.sort_unstable();
    let mut active = BTreeMap::<u32, u32>::new();
    for (_, start, y, bottom) in events {
        if start {
            if active
                .range(..=y)
                .next_back()
                .is_some_and(|(_, &end)| end > y)
                || active
                    .range(y..)
                    .next()
                    .is_some_and(|(&next, _)| next < bottom)
            {
                return Err(invalid());
            }
            active.insert(y, bottom);
        } else {
            active.remove(&y);
        }
    }
    Ok(bytes)
}

/// Atlas texel bounds precede both byte multiplication and any pixel allocation.
fn cell_bytes([x, y, width, height]: [u32; 4]) -> Result<usize, ArchiveError> {
    if width == 0
        || height == 0
        || x.checked_add(width)
            .is_none_or(|right| right > NAMETAG_ATLAS_SIDE)
        || y.checked_add(height)
            .is_none_or(|bottom| bottom > NAMETAG_ATLAS_SIDE)
    {
        return Err(invalid());
    }
    Ok(width as usize * height as usize * 4)
}

/// One named diagnostic covers malformed prepared tag geometry, never a missing legacy channel.
fn invalid() -> ArchiveError {
    ArchiveError::Invalid("invalid prepared world nametag data")
}

#[cfg(test)]
mod tests;
