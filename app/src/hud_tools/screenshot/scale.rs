//! GPU recording output: redirect the final composite, present it, then downscale before readback.

use std::sync::{Arc, Mutex};

use bevy::{
    camera::{NormalizedRenderTarget, RenderTarget},
    core_pipeline::blit::{BlitPipeline, BlitPipelineKey},
    prelude::*,
    render::{
        Extract, ExtractSchedule, GpuResourceAppExt, Render, RenderApp, RenderStartup,
        RenderSystems,
        camera::ExtractedCamera,
        render_asset::prepare_assets,
        render_resource::{binding_types::*, *},
        renderer::{RenderContext, RenderDevice, RenderGraph, RenderGraphSystems},
        texture::{GpuImage, OutputColorAttachment},
        view::{ViewTargetAttachments, prepare_view_attachments, prepare_view_targets},
    },
    window::PrimaryWindow,
};

use super::{FrameCapture, FrameCaptureSet, Screenshot, ScreenshotCaptured, offscreen};

const RETAIN_IDLE_FRAMES: u16 = 600;
type Status = Arc<Mutex<Result<(), String>>>;

#[derive(Resource, Default)]
struct Pending(Option<Request>);

#[derive(Clone)]
struct Request {
    source: RenderTarget,
    source_size: [u32; 2],
    output: Handle<Image>,
    status: Status,
}

#[derive(Resource, Default)]
struct Output {
    image: Option<Handle<Image>>,
    idle: u16,
}

#[derive(Resource, Default)]
struct Extracted(Option<(NormalizedRenderTarget, Request)>);

#[derive(Resource, Default)]
struct Source {
    texture: Option<(UVec2, TextureFormat, Texture, TextureView, TextureView)>,
    idle: u16,
}

#[derive(Resource, Default)]
struct Prepared(Option<Frame>);

#[derive(Resource)]
struct GraphInstalled;

struct Frame {
    rendered: OutputColorAttachment,
    display: OutputColorAttachment,
    output: NormalizedRenderTarget,
    display_pipeline: CachedRenderPipelineId,
    output_pipeline: CachedRenderPipelineId,
    display_binding: BindGroup,
    output_binding: BindGroup,
    status: Status,
}

/// Uses Bevy's blit shader with a filtering layout for a smooth recording-size reduction.
#[derive(Resource)]
struct ScalePipeline(BlitPipeline);

impl SpecializedRenderPipeline for ScalePipeline {
    type Key = TextureFormat;

    fn specialize(&self, format: Self::Key) -> RenderPipelineDescriptor {
        self.0.specialize(BlitPipelineKey {
            target_format: format,
            blend_state: None,
            samples: 1,
            source_space: None,
        })
    }
}

/// Installs a bounded output owner and a blit after every camera has finished its final composite.
pub(super) fn configure(app: &mut App) {
    app.init_resource::<Pending>()
        .init_resource::<Output>()
        .add_systems(
            Last,
            spawn
                .after(super::spawn_frame_capture)
                .in_set(FrameCaptureSet),
        );
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_app
        .configure_sets(
            Render,
            RenderSystems::PrepareViews.after(prepare_assets::<GpuImage>),
        )
        .init_resource::<Extracted>()
        .init_gpu_resource::<Source>()
        .init_gpu_resource::<Prepared>()
        .init_gpu_resource::<SpecializedRenderPipelines<ScalePipeline>>()
        .add_systems(
            RenderStartup,
            initialize.after(bevy::core_pipeline::blit::init_blit_pipeline),
        )
        .add_systems(ExtractSchedule, extract)
        .add_systems(
            Render,
            (
                prepare
                    .after(prepare_view_attachments)
                    .before(prepare_view_targets)
                    .in_set(RenderSystems::PrepareViews),
                release_display.in_set(RenderSystems::Cleanup),
            ),
        );
}

/// A single small image is reused; outstanding screenshots own independent GPU transfer buffers.
fn spawn(
    mut captures: ResMut<FrameCapture>,
    target: Res<offscreen::FrameTarget>,
    mut output: ResMut<Output>,
    mut pending: ResMut<Pending>,
    images: Option<ResMut<Assets<Image>>>,
    mut commands: Commands,
) {
    pending.0 = None;
    let Some(mut request) = captures.sized.take() else {
        output.idle = output.idle.saturating_add(1);
        if output.idle >= RETAIN_IDLE_FRAMES
            && let Some(image) = output.image.take()
            && let Some(mut images) = images
        {
            images.remove(image.id());
        }
        return;
    };
    output.idle = 0;
    let Some(mut images) = images else {
        for consumer in request.consumers {
            consumer(Err("The GPU recording output is unavailable.".into()));
        }
        return;
    };
    let size = UVec2::from_array(request.size);
    let mut changed = false;
    let image = output.image.get_or_insert_with(|| {
        changed = true;
        let mut texture =
            Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8Unorm, None);
        texture.data = None;
        images.add(texture)
    });
    if images
        .get(&*image)
        .is_some_and(|texture| texture.size() != size)
        && let Some(mut texture) = images.get_mut(&*image)
    {
        changed = true;
        texture.resize(Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        });
    }
    if changed {
        // Asset events flush before Last. This slot is skipped while the new GPU image is admitted.
        return;
    }
    let status = Arc::new(Mutex::new(Err(
        "The renderer did not produce the requested recording output.".into(),
    )));
    pending.0 = Some(Request {
        source: target
            .0
            .as_ref()
            .map_or_else(RenderTarget::default, |image| {
                RenderTarget::Image(image.clone().into())
            }),
        source_size: request.source,
        output: image.clone(),
        status: Arc::clone(&status),
    });
    commands.spawn(Screenshot::image(image.clone())).observe(
        move |mut captured: On<ScreenshotCaptured>| {
            let result = status.lock().expect("recording output status").clone();
            if let Err(error) = result {
                for consumer in request.consumers.drain(..) {
                    consumer(Err(error.clone()));
                }
            } else if let Some(last) = request.consumers.pop() {
                for consumer in request.consumers.drain(..) {
                    consumer(Ok(captured.image.clone()));
                }
                last(Ok(std::mem::take(&mut captured.event_mut().image)));
            }
        },
    );
}

/// Resolves the same primary window or hidden output that owns the ordinary shared screenshot.
fn extract(
    pending: Extract<Res<Pending>>,
    primary: Extract<Query<Entity, With<PrimaryWindow>>>,
    mut extracted: ResMut<Extracted>,
) {
    extracted.0 = pending.0.as_ref().and_then(|request| {
        request
            .source
            .normalize(primary.iter().next())
            .map(|source| (source, request.clone()))
    });
}

/// Reuses the shipped blit shader; only the owned filtering layout and sampler differ.
fn initialize(world: &mut World) {
    let device = world.resource::<RenderDevice>();
    let blit = world.resource::<BlitPipeline>();
    let pipeline = ScalePipeline(BlitPipeline {
        layout: BindGroupLayoutDescriptor::new(
            "recording scale layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        ),
        sampler: device.create_sampler(&SamplerDescriptor {
            label: Some("recording scale sampler"),
            min_filter: FilterMode::Linear,
            mag_filter: FilterMode::Linear,
            ..default()
        }),
        fullscreen_shader: blit.fullscreen_shader.clone(),
        fragment_shader: blit.fragment_shader.clone(),
    });
    world.insert_resource(pipeline);
    if world.contains_resource::<GraphInstalled>() {
        return;
    }
    world
        .try_schedule_scope(RenderGraph, |_, schedule| {
            schedule.add_systems(
                render
                    .after(bevy::core_pipeline::schedule::camera_driver)
                    .before(RenderGraphSystems::Submit)
                    .in_set(RenderGraphSystems::Render),
            );
        })
        .expect("recording output requires the camera render graph");
    world.insert_resource(GraphInstalled);
}

/// Redirects the existing final camera output, including direct HUD composites, without rerendering.
fn prepare(
    extracted: Res<Extracted>,
    mut source: ResMut<Source>,
    mut prepared: ResMut<Prepared>,
    mut attachments: ResMut<ViewTargetAttachments>,
    cameras: Query<&ExtractedCamera>,
    device: Res<RenderDevice>,
    pipeline: Res<ScalePipeline>,
    mut cache: ResMut<PipelineCache>,
    mut variants: ResMut<SpecializedRenderPipelines<ScalePipeline>>,
) {
    prepared.0 = None;
    let Some((target, request)) = &extracted.0 else {
        source.idle = source.idle.saturating_add(1);
        if source.idle >= RETAIN_IDLE_FRAMES {
            source.texture = None;
        }
        return;
    };
    source.idle = 0;
    let size = UVec2::from_array(request.source_size);
    let mut found = false;
    let mut resized = false;
    for camera in cameras
        .iter()
        .filter(|camera| camera.target.as_ref() == Some(target))
    {
        found = true;
        resized |= camera.physical_target_size != Some(size);
    }
    if !found || resized {
        *request.status.lock().expect("recording output status") =
            Err("The recording window changed size; stop and start a new recording.".into());
        return;
    }
    let Some(display) = attachments.get(target).cloned() else {
        return;
    };
    let format = display.view_format;
    if !matches!(
        format,
        TextureFormat::Rgba8Unorm
            | TextureFormat::Rgba8UnormSrgb
            | TextureFormat::Bgra8Unorm
            | TextureFormat::Bgra8UnormSrgb
    ) {
        *request.status.lock().expect("recording output status") =
            Err("This window's output format cannot be captured as SDR video.".into());
        return;
    }
    if source
        .texture
        .as_ref()
        .is_none_or(|(old_size, old_format, _, _, _)| *old_size != size || *old_format != format)
    {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("recording final composite"),
            size: Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: format.remove_srgb_suffix(),
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[format.add_srgb_suffix()],
        });
        let raw = texture.create_view(&default());
        let rendered = texture.create_view(&TextureViewDescriptor {
            format: Some(format),
            ..default()
        });
        source.texture = Some((size, format, texture, rendered, raw));
    }
    let (_, _, _, view, raw) = source.texture.as_ref().expect("recording output allocated");
    let rendered = OutputColorAttachment::new(view.clone(), format);
    let display_pipeline = variants.specialize(&cache, &pipeline, format);
    let output_pipeline = variants.specialize(&cache, &pipeline, TextureFormat::Rgba8Unorm);
    cache.block_on_render_pipeline(display_pipeline);
    cache.block_on_render_pipeline(output_pipeline);
    if cache.get_render_pipeline(display_pipeline).is_none()
        || cache.get_render_pipeline(output_pipeline).is_none()
    {
        *request.status.lock().expect("recording output status") =
            Err("The GPU recording scale pipeline is unavailable.".into());
        return;
    }
    prepared.0 = Some(Frame {
        rendered: rendered.clone(),
        display,
        output: NormalizedRenderTarget::Image(request.output.clone().into()),
        display_pipeline,
        output_pipeline,
        display_binding: pipeline.0.create_bind_group(&device, view, &cache),
        // The video retains the final output's bytes, including an unorm window's color encoding.
        output_binding: pipeline.0.create_bind_group(&device, raw, &cache),
        status: Arc::clone(&request.status),
    });
    attachments.insert(target.clone(), rendered);
}

/// Writes display and recording pixels before Bevy submits the small screenshot transfer.
fn render(
    prepared: Res<Prepared>,
    attachments: Res<ViewTargetAttachments>,
    cache: Res<PipelineCache>,
    mut context: RenderContext,
) {
    let Some(frame) = &prepared.0 else { return };
    if !frame.rendered.needs_present() {
        return;
    }
    let Some(display_pipeline) = cache.get_render_pipeline(frame.display_pipeline) else {
        return;
    };
    blit(
        &mut context,
        &frame.display,
        display_pipeline,
        &frame.display_binding,
    );
    let (Some(output), Some(output_pipeline)) = (
        attachments.get(&frame.output),
        cache.get_render_pipeline(frame.output_pipeline),
    ) else {
        return;
    };
    blit(&mut context, output, output_pipeline, &frame.output_binding);
    *frame.status.lock().expect("recording output status") = Ok(());
}

/// Draws one full-screen triangle into the retained output attachment at its own GPU dimensions.
fn blit(
    context: &mut RenderContext,
    target: &OutputColorAttachment,
    pipeline: &RenderPipeline,
    binding: &BindGroup,
) {
    let attachments = [Some(target.get_attachment(Some(LinearRgba::BLACK)))];
    let mut pass = context
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("recording output blit"),
            color_attachments: &attachments,
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, binding, &[]);
    pass.draw(0..3, 0..1);
}

/// Surface views are frame-owned and must be released before a later DXGI resize.
fn release_display(mut prepared: ResMut<Prepared>) {
    prepared.0 = None;
}
