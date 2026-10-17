use super::animate::{
    AtlasAnimations, BLEND_INSTANCE_ATTRIBUTES, BLEND_SOURCE_ENTRY, BlendInstance, BlendPipelines,
};
use super::pixels::{MipImages, build_images};
use super::plan::plan_atlas;
use super::tests::{FRAME_TICKS, expected, fixture, slot_rect};
use super::upload::{COLOR_STORAGE_FORMAT, PBR_FORMAT, upload_atlas};
use super::*;

/// Reads one level of a colour atlas back as encoded bytes.
fn read_level(gpu: &crate::gpu_snapshot::Gpu, texture: &wgpu::Texture, level: u32) -> MipImages {
    let (width, height) = (texture.width() >> level, texture.height() >> level);
    let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: level,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let mapped = buffer.slice(..).get_mapped_range().unwrap();
    let pixels = mapped
        .chunks_exact(row as usize)
        .flat_map(|line| &line[..width as usize * 4])
        .copied()
        .collect();
    let mut levels = vec![Vec::new(); level as usize];
    levels.push(pixels);
    MipImages {
        width: texture.width(),
        height: texture.height(),
        levels,
    }
}

fn decode(value: u8) -> f32 {
    let value = f32::from(value) / 255.0;
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn encode(value: f32) -> u8 {
    let value = if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The chunk shaders' frame mix: sampled (decoded) colour and linear alpha.
fn blended(current: &[u8], next: &[u8], weight: f32) -> Vec<u8> {
    current
        .as_chunks::<4>()
        .0
        .iter()
        .zip(next.as_chunks::<4>().0)
        .flat_map(|(a, b)| {
            let mut texel = [0; 4];
            for channel in 0..3 {
                let mixed = decode(a[channel]) * (1.0 - weight) + decode(b[channel]) * weight;
                texel[channel] = encode(mixed);
            }
            texel[3] = (f32::from(a[3]) * (1.0 - weight) + f32::from(b[3]) * weight).round() as u8;
            texel
        })
        .collect()
}

#[test]
fn animated_display_sprites_show_the_clock_frame_after_an_update() {
    let Some(gpu) = crate::gpu_snapshot::Gpu::for_fixture("shader-pack atlas animation update")
    else {
        return;
    };
    let fixture = fixture();
    let (plan, images) = fixture.with_inputs(None, |inputs| {
        let plan = plan_atlas(inputs, gpu.device.limits().max_texture_dimension_2d).unwrap();
        let images = build_images(inputs, &plan);
        (plan, images)
    });
    let uploaded = upload_atlas(&gpu.device, &gpu.queue, &plan, &images);
    let mut animations = AtlasAnimations::new(
        &uploaded,
        &fixture.animations,
        &fixture.frames,
        plan.animations.clone(),
        plan.mip_levels,
        [plan.atlas.width, plan.atlas.height],
    );
    let layout = gpu
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[BLEND_SOURCE_ENTRY],
        });
    let pipeline_layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader-pack frame blend"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blend.wgsl").into()),
        });
    let pipeline = |format: wgpu::TextureFormat| {
        gpu.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<BlendInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &BLEND_INSTANCE_ATTRIBUTES,
                    })],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fragment"),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
    };
    let (srgb, linear) = (pipeline(COLOR_STORAGE_FORMAT), pipeline(PBR_FORMAT));
    animations.bind_blend_sources(&gpu.device, &layout);
    let clock = ChunkAnimationClock::from_parts(FRAME_TICKS + 1, 0.5);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let pipelines = BlendPipelines {
        srgb: &srgb,
        linear: &linear,
    };
    assert_eq!(
        animations.encode(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            clock,
            Some(pipelines)
        ),
        2
    );
    gpu.queue.submit([encoder.finish()]);
    let plain = slot_rect(&plan, animation_sprite_slot(0));
    let blending = slot_rect(&plan, animation_sprite_slot(1));
    for level in 0..plan.mip_levels {
        let atlas = read_level(&gpu, &uploaded.atlas.color, level);
        let side = plain.side >> level;
        assert_eq!(
            atlas.read(level, plain),
            expected(0, 4, level, side),
            "mip {level}"
        );
        let wanted = blended(
            &expected(0, 4, level, side),
            &expected(0, 5, level, side),
            0.75,
        );
        for (got, want) in atlas.read(level, blending).iter().zip(&wanted) {
            assert!(got.abs_diff(*want) <= 1, "mip {level}: {got} vs {want}");
        }
    }
}
