//! The resources pulled pack pipelines bind in group 3 (`pull.rs`): the packed renderer's
//! arenas and tables, the atlas remap, and the attribute table and parameters the expansion
//! kernels otherwise receive per dispatch.

use std::hash::{BuildHasher, Hash, Hasher};

use bevy::platform::hash::FixedHasher;
use bevy::render::render_resource::{Buffer, BufferInitDescriptor, BufferUsages};
use bevy::render::renderer::{RenderDevice, RenderQueue};

use super::pull::PulledResource;
use crate::chunk::shaderpack_sources::PackedChunkBuffers;

/// Every resource a pulled stage may bind, with a key that changes whenever one does.
#[derive(Default)]
pub(crate) struct PulledArena {
    buffers: Vec<(PulledResource, Buffer)>,
    /// The material attribute table and the words it holds.
    attributes: Option<(Buffer, Vec<u32>)>,
    /// The parameters and the words they hold.
    params: Option<(Buffer, Vec<u32>)>,
    pub(crate) key: u64,
}

impl PulledArena {
    /// Whether every resource exists.
    pub(crate) fn ready(&self) -> bool {
        !self.buffers.is_empty()
    }

    pub(crate) fn buffer(&self, resource: PulledResource) -> Option<&wgpu::Buffer> {
        self.buffers
            .iter()
            .find(|(r, _)| *r == resource)
            .map(|(_, buffer)| &**buffer)
    }

    /// Brings the attribute table (`attributes`, two words per material) and the kernels'
    /// parameter words `params` up to date and collects this frame's arena buffers.
    pub(crate) fn update(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        arena: &PackedChunkBuffers<'_>,
        remap: &Buffer,
        attributes: &[u32],
        params: &[u32],
    ) {
        // A storage binding cannot be empty.
        let attributes = if attributes.is_empty() {
            vec![0; 2]
        } else {
            attributes.to_vec()
        };
        if self
            .attributes
            .as_ref()
            .is_none_or(|(_, held)| *held != attributes)
        {
            let buffer = device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("shader-pack pulled block attributes"),
                contents: bytemuck::cast_slice(&attributes),
                usage: BufferUsages::STORAGE,
            });
            self.attributes = Some((buffer, attributes));
        }
        match &mut self.params {
            Some((buffer, held)) => {
                if held != params {
                    queue.write_buffer(buffer, 0, bytemuck::cast_slice(params));
                    *held = params.to_vec();
                }
            }
            None => {
                let buffer = device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("shader-pack pulled parameters"),
                    contents: bytemuck::cast_slice(params),
                    usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                });
                self.params = Some((buffer, params.to_vec()));
            }
        }
        let (Some((attributes, _)), Some((params, _))) = (&self.attributes, &self.params) else {
            return;
        };
        self.buffers = vec![
            (PulledResource::Quads, arena.quads.clone()),
            (PulledResource::Origins, arena.origins.clone()),
            (PulledResource::Materials, arena.materials.clone()),
            (PulledResource::BiomeRecords, arena.biome_records.clone()),
            (PulledResource::BiomeTints, arena.biome_tints.clone()),
            (
                PulledResource::ModelTemplates,
                arena.model_templates.clone(),
            ),
            (
                PulledResource::GeometryStreams,
                arena.geometry_streams.clone(),
            ),
            (PulledResource::BiomeQuery, arena.world_uniform.clone()),
            (PulledResource::Attributes, attributes.clone()),
            (PulledResource::AtlasRemap, remap.clone()),
            (PulledResource::Params, params.clone()),
        ];
        let mut hasher = FixedHasher.build_hasher();
        for (_, buffer) in &self.buffers {
            buffer.id().hash(&mut hasher);
        }
        self.key = hasher.finish();
    }
}
