//! Samples the already rendered world before screen HUD/chat can modify its texture.

use super::*;
use bevy::render::{renderer::ViewQuery, view::ViewTarget};

pub(super) struct WorldFrame {
    pub(super) view: Entity,
    pub(super) size: UVec2,
    pub(super) output: NormalizedRenderTarget,
    pub(super) pipeline: CachedRenderPipelineId,
    pub(super) status: Status,
}

/// Copies only the admitted current view; absent/unsupported views leave the request failed.
pub(super) fn render(
    world: &World,
    view: ViewQuery<(&ViewTarget, &ExtractedCamera)>,
    mut context: RenderContext,
) {
    let entity = view.entity();
    let (target, camera) = view.into_inner();
    let Some((Frame::World(frame), _)) = world.resource::<Prepared>().0.as_ref() else {
        return;
    };
    if entity != frame.view {
        return;
    }
    if camera.physical_target_size != Some(frame.size)
        || camera.viewport.as_ref().is_some_and(|viewport| {
            viewport.physical_position != UVec2::ZERO || viewport.physical_size != frame.size
        })
    {
        *frame.status.lock().expect("recording output status") = Some(Err(
            "The recording camera no longer covers its original viewport.".into(),
        ));
        return;
    }
    let texture = target.main_texture();
    if texture.sample_count() != 1
        || !texture.usage().contains(TextureUsages::TEXTURE_BINDING)
        || !matches!(
            target.main_texture_format(),
            TextureFormat::Rgba8Unorm
                | TextureFormat::Rgba8UnormSrgb
                | TextureFormat::Bgra8Unorm
                | TextureFormat::Bgra8UnormSrgb
                | TextureFormat::Rgba16Float
        )
    {
        *frame.status.lock().expect("recording output status") = Some(Err(
            "The world image is not available for clean GPU recording.".into(),
        ));
        return;
    }
    let cache = world.resource::<PipelineCache>();
    let (Some(output), Some(pipeline)) = (
        world.resource::<ViewTargetAttachments>().get(&frame.output),
        cache.get_render_pipeline(frame.pipeline),
    ) else {
        return;
    };
    let binding = world.resource::<ScalePipeline>().0.create_bind_group(
        context.render_device(),
        target.main_texture_view(),
        cache,
    );
    blit(&mut context, output, pipeline, &binding);
    *frame.status.lock().expect("recording output status") = Some(Ok(()));
}
