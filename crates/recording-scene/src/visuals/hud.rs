//! Portable prepared HUD geometry and original-resolution font/image texels.

use crate::binary::{self, Reader};
use recording_studio::{
    archive::ArchiveError,
    object_store::{AssetId, AssetWriter},
};
use render_model::{
    MAX_UI_BATCHES, MAX_UI_DRAW_BYTES, MAX_UI_INDICES, MAX_UI_TEXTURE_LAYERS, MAX_UI_VERTICES,
    PreparedUiFrame, UiRenderBatch, UiRenderInput, UiRenderVertex, UiScissor, UiTextureCatalog,
    UiTextureFormat, UiTexturePage,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Write, sync::Arc};

const VERTEX_WORDS: usize = std::mem::size_of::<UiRenderVertex>() / 4;
const BATCH_WORDS: usize = 13;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Metadata {
    revision: u64,
    viewport: [u32; 2],
    safe_area: [u32; 4],
    time_seconds: f32,
    glint_strength: f32,
    pages: Vec<Page>,
    glint: Option<Page>,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    dimensions: [u32; 2],
    coverage: bool,
    pixels: AssetId,
}

#[derive(Default)]
pub(super) struct Loader {
    pages: Vec<Page>,
    textures: Option<Arc<UiTextureCatalog>>,
    glint: Option<(Page, Arc<UiTexturePage>)>,
}

/// Queued HUD frames retain only immutable prepared draws and their actual sampled texture owners.
pub(super) fn retained_bytes(frame: &PreparedUiFrame) -> usize {
    let input = &frame.input;
    std::mem::size_of::<PreparedUiFrame>()
        + std::mem::size_of_val(&*input.vertices)
        + std::mem::size_of_val(&*input.indices)
        + std::mem::size_of_val(&*input.batches)
        + input
            .textures
            .pages()
            .iter()
            .map(|page| page.pixels().len())
            .sum::<usize>()
        + frame.glint.as_ref().map_or(0, |page| page.pixels().len())
}

/// Only pages referenced by the admitted draw list are frozen; source font UVs keep native texels.
pub(super) fn store(
    store: &mut AssetWriter,
    frame: &PreparedUiFrame,
) -> Result<(Metadata, Vec<u8>), ArchiveError> {
    let input = &frame.input;
    input.validate().map_err(ui_error)?;
    let mut mapping = BTreeMap::new();
    let mut pages = Vec::new();
    let mut batches = Vec::with_capacity(input.batches.len());
    for batch in input.batches.iter() {
        let page = match mapping.get(&batch.texture_page) {
            Some(&page) => page,
            None => {
                let page = pages.len() as u32;
                pages.push(store_page(
                    store,
                    &input.textures.pages()[batch.texture_page as usize],
                )?);
                mapping.insert(batch.texture_page, page);
                page
            }
        };
        let mut batch = *batch;
        batch.texture_page = page;
        batches.push(batch);
    }
    // An empty publication still has a valid catalog; it draws nothing and invents no visible pixel.
    if pages.is_empty() {
        pages.push(store_page(store, &input.textures.pages()[0])?);
    }
    let metadata = Metadata {
        revision: input.revision,
        viewport: input.viewport_size,
        safe_area: input.safe_area,
        time_seconds: frame.time_seconds,
        glint_strength: frame.glint_strength,
        pages,
        glint: frame
            .glint
            .as_ref()
            .map(|page| store_page(store, page))
            .transpose()?,
    };
    let mut bytes = Vec::new();
    write_draws(&mut bytes, input, &batches)?;
    if bytes.len() > MAX_UI_DRAW_BYTES + 64 {
        return Err(ArchiveError::Quota);
    }
    Ok((metadata, bytes))
}

/// Writes every prepared field in the owning renderer's fixed POD vertex layout and explicit batch layout.
fn write_draws(
    out: &mut dyn Write,
    input: &UiRenderInput,
    batches: &[UiRenderBatch],
) -> Result<(), ArchiveError> {
    binary::packed(out, &input.vertices, |v| {
        bytemuck::cast::<UiRenderVertex, [u32; VERTEX_WORDS]>(*v)
    })?;
    binary::packed(out, &input.indices, |index| [*index])?;
    binary::packed(out, batches, |batch| {
        let UiRenderBatch {
            texture_page,
            scissor:
                UiScissor {
                    x,
                    y,
                    width,
                    height,
                },
            first_index,
            index_count,
            blend_mode,
            depth_test,
            depth_write,
            world_projection,
            isolated_depth_scope,
        } = *batch;
        [
            texture_page,
            x,
            y,
            width,
            height,
            first_index,
            index_count,
            blend_mode,
            depth_test,
            depth_write,
            world_projection,
            u32::from(isolated_depth_scope.is_some()),
            isolated_depth_scope.unwrap_or(0),
        ]
    })
}

/// Immutable source font and raster storage is shared with the journal worker without producer copies.
fn store_page(store: &mut AssetWriter, page: &UiTexturePage) -> Result<Page, ArchiveError> {
    Ok(Page {
        dimensions: page.dimensions(),
        coverage: page.format() == UiTextureFormat::Coverage,
        pixels: store.put_shared_pixels(&page.shared_pixels())?,
    })
}

/// Reconstructs a self-contained UI publication through the UI owner's texture and draw admission.
pub(super) fn load(
    cache: &mut Loader,
    metadata: Metadata,
    bytes: &[u8],
    mut pixels: impl FnMut(AssetId, usize) -> Result<Arc<[u8]>, ArchiveError>,
    budget: &mut usize,
    cancelled: &impl Fn() -> bool,
) -> Result<PreparedUiFrame, ArchiveError> {
    let m = metadata;
    if m.pages.is_empty()
        || m.pages.len() > MAX_UI_TEXTURE_LAYERS as usize
        || !m.time_seconds.is_finite()
        || !m.glint_strength.is_finite()
        || m.glint_strength < 0.0
    {
        return Err(ArchiveError::Invalid("invalid recorded HUD metadata"));
    }
    for page in m.pages.iter().chain(m.glint.iter()) {
        *budget = budget
            .checked_sub(page_bytes(page)?)
            .ok_or(ArchiveError::Quota)?;
    }
    let mut source = &bytes[..];
    let mut reader = Reader::with_limit(&mut source, MAX_UI_DRAW_BYTES + 64);
    let vertices = reader.packed_limit(MAX_UI_VERTICES, |w: [u32; VERTEX_WORDS]| {
        Some(bytemuck::cast::<_, UiRenderVertex>(w))
    })?;
    let indices = reader.packed_limit(MAX_UI_INDICES, |w: [u32; 1]| Some(w[0]))?;
    let batches = reader.packed_limit(MAX_UI_BATCHES, |w: [u32; BATCH_WORDS]| {
        let isolated_depth_scope = match w[11] {
            0 if w[12] == 0 => None,
            1 => Some(w[12]),
            _ => return None,
        };
        Some(UiRenderBatch {
            texture_page: w[0],
            scissor: UiScissor::new(w[1], w[2], w[3], w[4]),
            first_index: w[5],
            index_count: w[6],
            blend_mode: w[7],
            depth_test: w[8],
            depth_write: w[9],
            world_projection: w[10],
            isolated_depth_scope,
        })
    })?;
    drop(reader);
    binary::finish(&mut source)?;
    let textures = if cache.pages == m.pages {
        cache
            .textures
            .clone()
            .ok_or(ArchiveError::Invalid("missing cached HUD catalog"))?
    } else {
        let pages = m
            .pages
            .iter()
            .cloned()
            .map(|page| {
                if cancelled() {
                    return Err(ArchiveError::Cancelled);
                }
                load_page(page, &mut pixels)
            })
            .collect::<Result<Vec<_>, ArchiveError>>()?;
        let count = pages.len();
        // Demand fonts keep source-resolution texels and prepared UVs, without a live font carrier.
        let textures = Arc::new(UiTextureCatalog::new(pages, count).map_err(ui_error)?);
        cache.pages = m.pages;
        cache.textures = Some(Arc::clone(&textures));
        textures
    };
    let input = UiRenderInput {
        revision: m.revision,
        viewport_size: m.viewport,
        safe_area: m.safe_area,
        vertices: vertices.into(),
        indices: indices.into(),
        batches: batches.into(),
        textures,
    };
    input.validate().map_err(ui_error)?;
    let glint = m
        .glint
        .map(|page| {
            if let Some((old, raster)) = &cache.glint
                && old == &page
            {
                return Ok(Arc::clone(raster));
            }
            let raster = Arc::new(load_page(page.clone(), &mut pixels)?);
            cache.glint = Some((page, Arc::clone(&raster)));
            Ok::<_, ArchiveError>(raster)
        })
        .transpose()?;
    if cancelled() {
        return Err(ArchiveError::Cancelled);
    }
    Ok(PreparedUiFrame {
        input: Arc::new(input),
        glint,
        time_seconds: m.time_seconds,
        glint_strength: m.glint_strength,
    })
}

/// Native constructors enforce device-independent page extents before pixel objects can be allocated.
fn load_page(
    page: Page,
    pixels: &mut impl FnMut(AssetId, usize) -> Result<Arc<[u8]>, ArchiveError>,
) -> Result<UiTexturePage, ArchiveError> {
    let expected = page_bytes(&page)?;
    let pixels = pixels(page.pixels, expected)?;
    if page.coverage {
        UiTexturePage::coverage(page.dimensions, pixels)
    } else {
        UiTexturePage::owned(page.dimensions, pixels)
    }
    .map_err(ui_error)
}

/// Cached and newly decoded source pages consume the same admitted resident pixel budget.
fn page_bytes(page: &Page) -> Result<usize, ArchiveError> {
    let [width, height] = page.dimensions;
    if width == 0
        || height == 0
        || width > render_model::MAX_UI_TEXTURE_SIDE
        || height > render_model::MAX_UI_TEXTURE_SIDE
    {
        return Err(ArchiveError::Invalid("invalid recorded HUD texture extent"));
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|p| p.checked_mul(if page.coverage { 1 } else { 4 }))
        .ok_or(ArchiveError::Quota)
}

/// UI validation errors remain recording admission failures, without borrowing the live UI runtime.
fn ui_error(_: render_model::UiRenderRejectReason) -> ArchiveError {
    ArchiveError::Invalid("recorded HUD failed native UI admission")
}
