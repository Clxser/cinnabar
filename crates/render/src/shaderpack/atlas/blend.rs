//! Render-world pipelines for blending animation frames into the atlas.

use super::animate::{
    BLEND_INSTANCE_ATTRIBUTES, BLEND_SOURCE_ENTRY, BlendInstance, BlendPipelines,
};
use super::upload::{COLOR_STORAGE_FORMAT, PBR_FORMAT};
use bevy::{
    asset::{Handle, uuid_handle},
    mesh::VertexBufferLayout,
    prelude::{Commands, Res, Resource, Shader},
    render::render_resource::{
        BindGroupLayoutDescriptor, CachedRenderPipelineId, ColorTargetState, ColorWrites,
        FragmentState, PipelineCache, RenderPipelineDescriptor, TextureFormat, VertexState,
        VertexStepMode,
    },
};

pub(crate) const BLEND_SHADER: Handle<Shader> =
    uuid_handle!("cb81ab4c-7783-4291-a169-a0a9d1d70c72");

/// Queued blend pipelines and the frame-store layout they bind.
#[derive(Resource)]
pub(crate) struct AtlasBlendPipelines {
    pub layout: BindGroupLayoutDescriptor,
    srgb: CachedRenderPipelineId,
    linear: CachedRenderPipelineId,
}

impl AtlasBlendPipelines {
    /// Both pipelines once compiled.
    pub fn get<'a>(&self, cache: &'a PipelineCache) -> Option<BlendPipelines<'a>> {
        Some(BlendPipelines {
            srgb: cache.get_render_pipeline(self.srgb)?,
            linear: cache.get_render_pipeline(self.linear)?,
        })
    }
}

fn descriptor(
    layout: &BindGroupLayoutDescriptor,
    format: TextureFormat,
) -> RenderPipelineDescriptor {
    RenderPipelineDescriptor {
        label: Some("shader-pack animated sprite blend".into()),
        layout: vec![layout.clone()],
        vertex: VertexState {
            shader: BLEND_SHADER,
            entry_point: Some("vertex".into()),
            buffers: vec![VertexBufferLayout {
                array_stride: size_of::<BlendInstance>() as u64,
                step_mode: VertexStepMode::Instance,
                attributes: BLEND_INSTANCE_ATTRIBUTES.to_vec(),
            }],
            ..Default::default()
        },
        fragment: Some(FragmentState {
            shader: BLEND_SHADER,
            entry_point: Some("fragment".into()),
            targets: vec![Some(ColorTargetState {
                format,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Queues the colour (sRGB storage) and LabPBR (linear) blend pipelines.
pub(crate) fn init_blend_pipelines(mut commands: Commands, cache: Res<PipelineCache>) {
    let layout =
        BindGroupLayoutDescriptor::new("shader-pack frame blend sources", &[BLEND_SOURCE_ENTRY]);
    commands.insert_resource(AtlasBlendPipelines {
        srgb: cache.queue_render_pipeline(descriptor(&layout, COLOR_STORAGE_FORMAT)),
        linear: cache.queue_render_pipeline(descriptor(&layout, PBR_FORMAT)),
        layout,
    });
}
