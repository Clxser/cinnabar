//! Immutable addressable actor geometry, shared by many pose records.

use std::sync::Arc;

use recording_studio::{
    archive::ArchiveError,
    object_store::{AssetId, AssetReader, AssetWriter},
};
use render_model::{
    ActorRigGeometrySpan, ActorRigVertex, ActorRigVertexSegments, actor_catalog_budget_bytes,
};

use crate::binary::{self, Reader};

use super::{io_error, magic};

const MAGIC: [u8; 8] = *b"CINAGE01";
pub(super) const MAX_BYTES: usize = actor_catalog_budget_bytes(u32::MAX) + 16 * 1024 * 1024;
const MAX_TABLE_BYTES: usize = MAX_BYTES - actor_catalog_budget_bytes(u32::MAX);
const VERTEX_WORDS: usize = std::mem::size_of::<ActorRigVertex>() / std::mem::size_of::<u32>();

pub(crate) struct LoadedGeometry {
    pub vertices: ActorRigVertexSegments,
    pub spans: Arc<[ActorRigGeometrySpan]>,
    pub bone_counts: Vec<u32>,
}

/// Writes canonical little-endian POD words rather than platform-dependent struct bytes.
pub(crate) fn store(
    store: &mut AssetWriter,
    vertices: &ActorRigVertexSegments,
    spans: &[ActorRigGeometrySpan],
) -> Result<AssetId, ArchiveError> {
    store.put(MAX_BYTES as u64, |output| {
        output
            .write_all(&MAGIC)
            .and_then(|()| output.write_all(&vertices.epoch.to_le_bytes()))
            .map_err(io_error)?;
        binary::u32(
            output,
            u32::try_from(vertices.len()).map_err(|_| ArchiveError::Quota)?,
        )?;
        binary::packed(output, spans, |span| [span.first_vertex, span.vertex_count])?;
        let offsets = vertices
            .offsets
            .iter()
            .map(|offset| u32::try_from(*offset))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ArchiveError::Quota)?;
        binary::packed(output, &offsets, |offset| [*offset])?;
        binary::u32(
            output,
            u32::try_from(vertices.segments.len()).map_err(|_| ArchiveError::Quota)?,
        )?;
        for segment in vertices.segments.iter() {
            binary::packed(output, segment, |vertex| {
                bytemuck::cast::<ActorRigVertex, [u32; VERTEX_WORDS]>(*vertex)
            })?;
        }
        Ok(())
    })
}

/// Every span and referenced vertex is admitted once, before per-frame pose or bone cross-references.
pub(crate) fn load(
    store: &AssetReader,
    id: AssetId,
    cancelled: &impl Fn() -> bool,
) -> Result<LoadedGeometry, ArchiveError> {
    store.read_cancellable(id, MAX_BYTES as u64, cancelled, |input| {
        magic(input, MAGIC)?;
        let mut epoch = [0; 8];
        input.read_exact(&mut epoch).map_err(io_error)?;
        let mut reader = Reader::with_limit(input, MAX_BYTES - 16);
        let len = reader.u32()? as usize;
        let spans = reader.packed_limit(
            MAX_TABLE_BYTES / std::mem::size_of::<ActorRigGeometrySpan>(),
            |words: [u32; 2]| {
                Some(ActorRigGeometrySpan {
                    first_vertex: words[0],
                    vertex_count: words[1],
                })
            },
        )?;
        let offsets = reader.packed_limit(
            MAX_TABLE_BYTES / std::mem::size_of::<Arc<[ActorRigVertex]>>(),
            |words: [u32; 1]| Some(words[0] as usize),
        )?;
        let count = reader.u32()? as usize;
        if count != offsets.len() {
            return Err(ArchiveError::Invalid(
                "recorded actor vertex pages do not match their address table",
            ));
        }
        let mut segments = Vec::with_capacity(count);
        for _ in 0..count {
            let vertices = reader.packed(|words: [u32; VERTEX_WORDS]| {
                Some(bytemuck::cast::<[u32; VERTEX_WORDS], ActorRigVertex>(words))
            })?;
            segments.push(Arc::<[ActorRigVertex]>::from(vertices));
        }
        drop(reader);
        binary::finish(input)?;
        let vertices = ActorRigVertexSegments::from_recorded_pages(
            u64::from_le_bytes(epoch),
            segments.into(),
            offsets.into(),
            len,
        )
        .ok_or(ArchiveError::Invalid(
            "invalid recorded actor geometry addresses",
        ))?;
        let mut bone_counts = Vec::with_capacity(spans.len());
        for span in &spans {
            if span.vertex_count == 0 && span.first_vertex as usize <= vertices.len() {
                bone_counts.push(0);
                continue;
            }
            let Some(slice) = vertices.span(*span) else {
                return Err(ArchiveError::Invalid(
                    "actor geometry span exceeds its immutable vertex page",
                ));
            };
            let mut bones = 0_u32;
            for vertex in slice {
                if vertex
                    .position
                    .iter()
                    .chain(vertex.normal.iter())
                    .chain(vertex.uv.iter())
                    .chain(vertex.back_uv.iter())
                    .chain(vertex.glint_origin.iter())
                    .any(|value| !value.is_finite())
                    || !vertex.surface.is_valid()
                {
                    return Err(ArchiveError::Invalid("invalid recorded actor vertex"));
                }
                bones = bones.max(
                    vertex
                        .bone_index
                        .checked_add(1)
                        .ok_or(ArchiveError::Invalid("actor vertex bone index overflow"))?,
                );
            }
            bone_counts.push(bones);
        }
        Ok(LoadedGeometry {
            vertices,
            spans: spans.into(),
            bone_counts,
        })
    })
}
