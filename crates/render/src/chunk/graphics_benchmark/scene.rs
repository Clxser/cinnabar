//! Original terrain and bindings for the production packed-terrain pipeline.

use crate::chunk::{
    FALLBACK_BIOME_RECORD, MaterialGpu, STATIC_QUAD_INDICES, prepare_biome_tint_entries,
};
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        view::{ColorGradingUniform, ViewUniform},
    },
};
use meshing::{BlockClassifier, mesh_sub_chunk_in_neighbourhood, synthetic_terrain};
use std::collections::BTreeMap;

const SIDE: usize = 16;
pub(super) const SECTIONS: usize = SIDE * SIDE;

/// A fixed scene with original textures, isolated from packs and world publication.
pub(super) struct Scene {
    pub groups: [BindGroup; 2],
    pub offsets: Buffer,
    pub indices: Buffer,
    pub ranges: Vec<std::ops::Range<u32>>,
}

/// Uploads fixture bytes with the usage declared by the real layout.
fn buffer(device: &RenderDevice, bytes: &[u8], usage: BufferUsages) -> Buffer {
    device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("graphics benchmark fixture"),
        contents: bytes,
        usage,
    })
}

/// Builds the view at the requested physical resolution, looking over the fixed terrain.
fn view(resolution: [u32; 2]) -> ViewUniform {
    let position = Vec3::new(128.0, 90.0, 300.0);
    let view_from_world =
        glam::camera::rh::view::look_at_mat4(position, Vec3::new(128.0, 0.0, 100.0), Vec3::Y);
    let clip_from_view = glam::camera::rh::proj::directx::perspective_infinite_reverse(
        70_f32.to_radians(),
        resolution[0] as f32 / resolution[1] as f32,
        0.1,
    );
    let clip_from_world = clip_from_view * view_from_world;
    ViewUniform {
        clip_from_world,
        unjittered_clip_from_world: clip_from_world,
        world_from_clip: clip_from_world.inverse(),
        world_from_view: view_from_world.inverse(),
        view_from_world,
        clip_from_view,
        view_from_clip: clip_from_view.inverse(),
        world_position: position,
        lod_view_world_position: position,
        exposure: 1.0,
        viewport: Vec4::new(0.0, 0.0, resolution[0] as f32, resolution[1] as f32),
        main_pass_viewport: Vec4::new(0.0, 0.0, resolution[0] as f32, resolution[1] as f32),
        frustum: [Vec4::ZERO; 6],
        color_grading: ColorGradingUniform {
            balance: Mat3::IDENTITY,
            saturation: Vec3::ONE,
            contrast: Vec3::ONE,
            gamma: Vec3::ONE,
            gain: Vec3::ONE,
            lift: Vec3::ZERO,
            midtone_range: Vec2::new(0.2, 0.7),
            exposure: 0.0,
            hue: 0.0,
            post_saturation: 1.0,
        },
        mip_bias: 0.0,
        frame_count: 0,
    }
}

impl Scene {
    /// Uses the production descriptors to derive binding types and minimum sizes.
    pub fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        cache: &PipelineCache,
        resolution: [u32; 2],
    ) -> Self {
        let assets = synthetic_terrain::terrain_assets();
        let section = synthetic_terrain::mixed_chunk();
        let mesh = mesh_sub_chunk_in_neighbourhood(
            &BlockClassifier::new(synthetic_terrain::AIR),
            &assets,
            assets::NetworkIdMode::Sequential,
            &world::MeshNeighbourhood::new(&section),
        );
        let mut quads = Vec::new();
        let mut lights = Vec::new();
        let mut origins = Vec::new();
        let mut ranges = Vec::new();
        let mut offsets = Vec::new();
        for section in 0..SECTIONS {
            let start = quads.len() as u32;
            quads.extend(mesh.cube_quads().iter().flat_map(|quad| quad.words()));
            // PackedQuad is two words; the draw range and origin name records, not words.
            let start = start / 2;
            ranges.push(start..quads.len() as u32 / 2);
            let light_start = lights.len() as u32 / 2;
            lights.extend(mesh.cube_lighting().iter().flat_map(|light| {
                let s = light.samples();
                [
                    u32::from(s[0]) | (u32::from(s[1]) << 16),
                    u32::from(s[2]) | (u32::from(s[3]) << 16),
                ]
            }));
            origins.push([
                (section % SIDE * 16) as u32,
                0,
                (section / SIDE * 16) as u32,
                0,
                start,
                light_start,
                0,
                0,
            ]);
            offsets.extend((0..4_u32).map(|corner| [(section as u32) * 4 + corner, 0]));
        }
        let materials: Vec<_> = assets
            .materials()
            .iter()
            .map(|material| MaterialGpu {
                texture: 0,
                flags: material.flags,
                animation: assets::NO_ANIMATION,
                variation_start: 0,
                variation_count: 0,
                variation_weight: 0,
            })
            .collect();
        let atmosphere = render_scene::AtmosphereFrame::default();
        let mut encoded = encase::UniformBuffer::new(Vec::new());
        encoded
            .write(&view(resolution))
            .expect("benchmark view uniform");
        let mut bytes = BTreeMap::from([
            (0, encoded.into_inner()),
            (1, bytemuck::cast_slice(&quads).to_vec()),
            (2, bytemuck::cast_slice(&origins).to_vec()),
            (7, bytemuck::cast_slice(&FALLBACK_BIOME_RECORD).to_vec()),
            (
                8,
                bytemuck::cast_slice(&prepare_biome_tint_entries(&[
                    render_scene::BiomeTint::default(),
                ]))
                .to_vec(),
            ),
            (13, bytemuck::cast_slice(&lights).to_vec()),
            (15, bytemuck::bytes_of(&atmosphere).to_vec()),
            (
                crate::material_shader::BIOME_QUERY_TABLES_BINDING,
                bytemuck::cast_slice(&meshing::biome_lattice::query_table_words()).to_vec(),
            ),
            (3, bytemuck::cast_slice(&materials).to_vec()),
        ]);
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("original benchmark terrain"),
            size: Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &[
                85, 120, 70, 255, 95, 135, 80, 255, 95, 135, 80, 255, 85, 120, 70, 255,
            ],
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(2),
            },
            texture.size(),
        );
        let texture = texture.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            ..default()
        });
        let sampler = device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..default()
        });
        let terrain = bindings(
            device,
            cache,
            &super::super::pipeline::layouts::opaque_chunk_bind_group_layout(),
            &mut bytes,
            &texture,
            &sampler,
        );
        let mut light_bytes = BTreeMap::from([
            (0, bytemuck::cast_slice(&[[1.0_f32; 4]; 256]).to_vec()),
            (1, bytemuck::bytes_of(&atmosphere).to_vec()),
        ]);
        let lighting = bindings(
            device,
            cache,
            &crate::lighting::layout(),
            &mut light_bytes,
            &texture,
            &sampler,
        );
        Self {
            groups: [terrain, lighting],
            offsets: buffer(device, bytemuck::cast_slice(&offsets), BufferUsages::VERTEX),
            indices: buffer(
                device,
                bytemuck::cast_slice(&STATIC_QUAD_INDICES),
                BufferUsages::INDEX,
            ),
            ranges,
        }
    }
}

/// Derives entries from the actual terrain layout rather than duplicating its binding list.
fn bindings(
    device: &RenderDevice,
    cache: &PipelineCache,
    layout: &BindGroupLayoutDescriptor,
    bytes: &mut BTreeMap<u32, Vec<u8>>,
    texture: &TextureView,
    sampler: &Sampler,
) -> BindGroup {
    let buffers: BTreeMap<_, _> = layout
        .entries
        .iter()
        .filter_map(|entry| {
            if let BindingType::Buffer {
                ty,
                min_binding_size,
                ..
            } = entry.ty
            {
                let data = bytes.entry(entry.binding).or_default();
                data.resize(
                    data.len()
                        .max(min_binding_size.map_or(16, |size| size.get() as usize)),
                    0,
                );
                let usage = if ty == BufferBindingType::Uniform {
                    BufferUsages::UNIFORM
                } else {
                    BufferUsages::STORAGE
                };
                Some((entry.binding, buffer(device, data, usage)))
            } else {
                None
            }
        })
        .collect();
    let entries: Vec<_> = layout
        .entries
        .iter()
        .map(|entry| BindGroupEntry {
            binding: entry.binding,
            resource: match entry.ty {
                BindingType::Buffer { .. } => buffers[&entry.binding].as_entire_binding(),
                BindingType::Texture { .. } => BindingResource::TextureView(texture),
                BindingType::Sampler(_) => BindingResource::Sampler(sampler),
                _ => panic!("unsupported terrain benchmark binding"),
            },
        })
        .collect();
    device.create_bind_group(
        "graphics benchmark scene",
        &cache.get_bind_group_layout(layout),
        &entries,
    )
}
