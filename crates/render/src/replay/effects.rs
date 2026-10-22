//! Shared GPU composition for native replay and raster-video frame effects.

use std::sync::Arc;

use bevy::render::{
    render_resource::{
        Extent3d, Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
        TextureViewDescriptor,
    },
    renderer::RenderDevice,
};
use recording_studio::{MediaTime, effect::EffectSample};
use render_scene::ui_preview::{PreviewImage, PreviewTarget};

use super::{ReplayRenderContext, targets::Targets};

mod pipelines;

/// The blur grid is independent of output resolution, preserving normalized appearance and bounded work.
const BLOOM_MAXIMUM: [u32; 2] = [384, 256];

macro_rules! parameters {
    ($($field:ident),+ $(,)?) => {
        #[repr(C)]
        #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
        struct Parameters { $($field: [f32; 4]),+ }
        impl Parameters {
            const WGSL: &str = concat!("struct Parameters {", $(stringify!($field), ": vec4<f32>,",)+ "};");
        }
    };
}
parameters!(tone, overlay, motion, step, color, detail);

pub struct FrameEffects {
    context: ReplayRenderContext,
    pipelines: Option<pipelines::Pipelines>,
    surfaces: Option<Surfaces>,
}

struct Surfaces {
    size: [u32; 2],
    targets: Targets,
    upload: Option<Texture>,
    black: Texture,
    bloom: Option<Bloom>,
}

struct Bloom {
    size: [u32; 2],
    extract: Texture,
    horizontal: Texture,
}

impl FrameEffects {
    /// Retains only shared device handles until a frame actually needs effects.
    pub fn new(context: ReplayRenderContext) -> Self {
        Self {
            context,
            pipelines: None,
            surfaces: None,
        }
    }

    /// Native camera effects were applied before rendering; post effects operate on the resulting image.
    pub fn native(
        &mut self,
        image: Arc<PreviewImage>,
        effects: EffectSample,
        at: MediaTime,
    ) -> Result<Option<Arc<PreviewImage>>, String> {
        self.composed(image, effects, at, true)
    }

    /// Final composition preserves image motion only when source cameras have not already applied it.
    pub fn composed(
        &mut self,
        image: Arc<PreviewImage>,
        effects: EffectSample,
        at: MediaTime,
        camera_applied: bool,
    ) -> Result<Option<Arc<PreviewImage>>, String> {
        let effects = if camera_applied {
            effects.without_camera()
        } else {
            effects
        };
        if effects.neutral() {
            return Ok(Some(image));
        }
        if image.target.device().wgpu_device() != self.context.device.wgpu_device() {
            return Err("The effects input belongs to another graphics device.".into());
        }
        let size = image.target.size();
        self.prepare(size, effects.bloom > 0.0)?;
        let Some(target) = self
            .surfaces
            .as_mut()
            .expect("effects surfaces admitted")
            .targets
            .acquire(&self.context.device)?
        else {
            return Ok(None);
        };
        let view = image
            .target
            .texture()
            .create_view(&TextureViewDescriptor::default());
        self.draw(&view, effects, at, Some(image), target)
    }

    /// A video frame is uploaded once; GPU effects feed the UI or export readback without CPU filtering.
    pub fn raster(
        &mut self,
        size: [u32; 2],
        rgba8: &[u8],
        effects: EffectSample,
        at: MediaTime,
    ) -> Result<Option<Arc<PreviewImage>>, String> {
        if recording_studio::video::frame_byte_len(size)? != rgba8.len() {
            return Err("The effects input has invalid pixel storage.".into());
        }
        self.prepare(size, effects.bloom > 0.0)?;
        let Some(target) = self
            .surfaces
            .as_mut()
            .expect("effects surfaces admitted")
            .targets
            .acquire(&self.context.device)?
        else {
            return Ok(None);
        };
        let surfaces = self.surfaces.as_mut().expect("effects surfaces admitted");
        if surfaces.upload.is_none() {
            surfaces.upload = Some(texture(
                &self.context.device,
                size,
                TextureFormat::Rgba8UnormSrgb,
                TextureUsages::COPY_DST,
            ));
        }
        let upload = surfaces.upload.as_ref().expect("raster upload admitted");
        self.context.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: upload,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba8,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 4),
                rows_per_image: Some(size[1]),
            },
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        let view = upload.create_view(&TextureViewDescriptor::default());
        self.draw(&view, effects, at, None, target)
    }

    /// Targets and scratch have independent bounds. Shader validation failures become worker errors.
    fn prepare(&mut self, size: [u32; 2], bloom: bool) -> Result<(), String> {
        if let Some(error) = self.context.faults.error() {
            return Err(error);
        }
        if self.pipelines.is_none() {
            self.pipelines = Some(pipelines::Pipelines::new(&self.context.device)?);
        }
        if self
            .surfaces
            .as_ref()
            .is_none_or(|surfaces| surfaces.size != size)
        {
            let targets = Targets::new(&self.context.device, size)?;
            let black = texture(
                &self.context.device,
                [1, 1],
                TextureFormat::Rgba8UnormSrgb,
                TextureUsages::COPY_DST,
            );
            self.context.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &black,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &[0; 4],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            self.surfaces = Some(Surfaces {
                size,
                targets,
                upload: None,
                black,
                bloom: None,
            });
        }
        let surfaces = self.surfaces.as_mut().expect("effects surfaces admitted");
        if bloom && surfaces.bloom.is_none() {
            let size = recording_studio::preview::fit_size(size, BLOOM_MAXIMUM)?;
            surfaces.bloom = Some(Bloom {
                size,
                extract: texture(
                    &self.context.device,
                    size,
                    TextureFormat::Rgba16Float,
                    TextureUsages::empty(),
                ),
                horizontal: texture(
                    &self.context.device,
                    size,
                    TextureFormat::Rgba16Float,
                    TextureUsages::empty(),
                ),
            });
        }
        Ok(())
    }

    /// Queue ordering protects shared blur/upload scratch; image leases protect displayed output allocations.
    fn draw(
        &mut self,
        source: &wgpu::TextureView,
        effects: EffectSample,
        at: MediaTime,
        held: Option<Arc<PreviewImage>>,
        target: Arc<PreviewTarget>,
    ) -> Result<Option<Arc<PreviewImage>>, String> {
        let surfaces = self.surfaces.as_mut().expect("effects surfaces admitted");
        let bloom_size = surfaces.bloom.as_ref().map_or([1, 1], |bloom| bloom.size);
        let [scale, shift_x, shift_y] = effects.image_motion(at);
        let EffectSample {
            zoom_degrees: _,
            shake_degrees: _,
            exposure_stops,
            saturation_delta,
            vignette,
            bloom,
            flash,
            letterbox,
            fade,
            contrast,
            warmth,
            tint,
            grain,
            chromatic,
            soft_focus,
            sharpen,
        } = effects;
        let parameters = Parameters {
            tone: [
                exposure_stops.exp2() as f32,
                (1.0 + saturation_delta) as f32,
                vignette as f32,
                bloom as f32,
            ],
            overlay: [flash as f32, letterbox as f32, fade as f32, 0.0],
            motion: [
                scale,
                shift_x,
                shift_y,
                surfaces.size[0] as f32 / surfaces.size[1] as f32,
            ],
            step: [
                1.0 / bloom_size[0] as f32,
                1.0 / bloom_size[1] as f32,
                1.0 / surfaces.size[0] as f32,
                1.0 / surfaces.size[1] as f32,
            ],
            color: [contrast as f32, warmth as f32, tint as f32, grain as f32],
            detail: [
                chromatic as f32,
                soft_focus as f32,
                sharpen as f32,
                (at.0 % 1_000_000_000) as f32 / 1_000_000.0,
            ],
        };
        let pipelines = self.pipelines.as_ref().expect("effects pipelines admitted");
        self.context
            .queue
            .write_buffer(&pipelines.parameters, 0, bytemuck::bytes_of(&parameters));
        let black = surfaces
            .black
            .create_view(&TextureViewDescriptor::default());
        let mut encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Studio frame effects"),
                });
        let glow = if bloom > 0.0 {
            let bloom = surfaces.bloom.as_ref().expect("bloom scratch admitted");
            let extract = bloom.extract.create_view(&TextureViewDescriptor::default());
            let horizontal = bloom
                .horizontal
                .create_view(&TextureViewDescriptor::default());
            pipelines.pass(
                &self.context.device,
                &mut encoder,
                pipelines::Pass::Extract,
                source,
                &black,
                &extract,
            );
            pipelines.pass(
                &self.context.device,
                &mut encoder,
                pipelines::Pass::Horizontal,
                source,
                &extract,
                &horizontal,
            );
            horizontal
        } else {
            black
        };
        let output = target
            .texture()
            .create_view(&TextureViewDescriptor::default());
        pipelines.pass(
            &self.context.device,
            &mut encoder,
            pipelines::Pass::Grade,
            source,
            &glow,
            &output,
        );
        self.context.queue.submit([encoder.finish()]);
        let submitted = Arc::clone(&target);
        self.context.queue.on_submitted_work_done(move || {
            drop(submitted);
            drop(held);
        });
        Ok(Some(Arc::new(PreviewImage { target })))
    }
}

/// All surfaces are single-sample, single-layer and explicitly typed for their sampling/attachment use.
fn texture(
    device: &RenderDevice,
    size: [u32; 2],
    format: TextureFormat,
    extra: TextureUsages,
) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: Some("Studio effects surface"),
        size: Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | extra,
        view_formats: &[],
    })
}
