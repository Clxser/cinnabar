//! The transparent mask hides a slot only when no depth-tested sample of its bounds survives
//! this frame's opaque depth: single- and multi-sampled, at the viewport edge, around the eye,
//! and through other transparent bounds, which never occlude.

use super::*;
use direct::{bit, pyramid_levels};
use kernels::TransparentMask;
use model::reference_transparent_occluded;

/// Draws each of `boxes` against `target`'s depth without writing it, in its own occlusion
/// query, and returns the boxes with at least one passing sample. Boxes never hide each other,
/// as transparent terrain never hides what draws after it from the pyramid.
fn passing_boxes(gpu: &Gpu, target: &Target, view: &[f32], boxes: &[[f32; 8]]) -> BTreeSet<usize> {
    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(hiz::SCENE_SHADER.into()),
        });
    let pipeline = gpu
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("box_vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: target.samples,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("colour_fragment"),
                compilation_options: Default::default(),
                targets: &[Some(target.color.format().into())],
            }),
            multiview_mask: None,
            cache: None,
        });
    let view = gpu.buffer(view, wgpu::BufferUsages::UNIFORM);
    let items = gpu.buffer(boxes.as_flattened(), wgpu::BufferUsages::STORAGE);
    let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: view.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: items.as_entire_binding(),
            },
        ],
    });
    let count = boxes.len() as u32;
    let queries = gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
        label: None,
        ty: wgpu::QueryType::Occlusion,
        count,
    });
    let resolved = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(count) * 8,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let color = target.color.create_view(&Default::default());
    let depth = target.depth.create_view(&Default::default());
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Discard,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: Some(&queries),
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        for index in 0..count {
            pass.begin_occlusion_query(index);
            pass.draw(0..36, index..index + 1);
            pass.end_occlusion_query();
        }
    }
    encoder.resolve_query_set(&queries, 0..count, &resolved, 0);
    gpu.queue.submit([encoder.finish()]);
    let samples = read_buffer(gpu, &resolved, u64::from(count) * 8);
    bytemuck::cast_slice::<u8, u64>(&samples)
        .iter()
        .enumerate()
        .filter(|(_, passed)| **passed != 0)
        .map(|(index, _)| index)
        .collect()
}

/// Wherever rounding cannot flip a decision the mask equals the CPU reference, and it never
/// hides a slot whose bounds pass a depth-tested sample.
#[test]
fn transparent_mask_hides_only_bounds_no_depth_tested_sample_reaches() {
    let Some(gpu) = Gpu::for_fixture("terrain transparent occlusion") else {
        return;
    };
    let eye = Vec3::new(8.25, 72.5, 8.75);
    let wall = [-40.0, 60.0, -24.0, 0.0, 56.0, 84.0, -23.0, 0.0];
    let mut random = Lcg(31);
    let mut records = Vec::new();
    let mut boxes = Vec::new();
    for x in -6..=6 {
        for y in 3..=5 {
            for z in -6..=1 {
                let slot = records.len() as u32;
                // Liquid cells, model overhangs, and slots that draw nothing transparent.
                let transparent = match random.next(4) {
                    0 => None,
                    1 => Some([[-3; 3], [19; 3]]),
                    _ => {
                        let low = [random.next(12), random.next(14), random.next(12)]
                            .map(|value| value as i32);
                        Some([low, low.map(|value| value + 1 + random.next(4) as i32)])
                    }
                };
                let origin = [x * 16, y * 16, z * 16];
                records.push(
                    CullRecord::new(&CullRecordSource {
                        origin,
                        base_vertex: slot as i32 * 4,
                        bounds: [[0; 3], [16; 3]],
                        transparent,
                        ..Default::default()
                    })
                    .unwrap(),
                );
                let [low, high] = transparent.unwrap_or([[0; 3], [0; 3]]);
                let world = |local: [i32; 3]| {
                    std::array::from_fn::<f32, 3, _>(|axis| (origin[axis] + local[axis]) as f32)
                };
                let (lo, hi) = (world(low), world(high));
                boxes.push([lo[0], lo[1], lo[2], 0.0, hi[0], hi[1], hi[2], 0.0]);
            }
        }
    }
    // The eye's own sub-chunk crosses the near plane and must never be hidden.
    let containing = records
        .iter()
        .position(|record| record.origin == [0, 64, 0])
        .unwrap();
    records[containing] = CullRecord::new(&CullRecordSource {
        origin: [0, 64, 0],
        base_vertex: containing as i32 * 4,
        bounds: [[0; 3], [16; 3]],
        transparent: Some([[0; 3], [16; 3]]),
        ..Default::default()
    })
    .unwrap();
    boxes[containing] = [0.0, 64.0, 0.0, 0.0, 16.0, 80.0, 16.0, 0.0];
    let culler = Culler::new(&gpu, &records, &enabled_words(|_| true, records.len()));
    let size = TransparentMask::size(
        &gpu.device.limits(),
        culler.kernels.workgroup,
        culler.storage.capacity,
    )
    .unwrap();
    let mask = TransparentMask::new(&gpu.device, size, true);
    assert!(mask.words() as usize * 32 >= records.len());
    let cameras = [
        camera(eye, Vec3::new(10.0, 70.0, -40.0)),
        camera(eye, Vec3::new(-30.0, 66.0, -40.0)),
        camera(Vec3::new(-20.5, 75.0, -4.0), Vec3::new(20.0, 68.0, -60.0)),
        camera(Vec3::new(30.5, 90.0, 20.25), Vec3::new(0.0, 64.0, -40.0)),
    ];
    let (mut decided, mut total, mut hidden, mut passing) = (0, 0, 0, 0);
    for samples in [1, 4] {
        for camera in &cameras {
            let view = camera.clip_from_world.to_cols_array();
            let target = Target::new(&gpu, wgpu::TextureFormat::Rgba8Unorm, samples);
            hiz::render_scene(
                &gpu,
                &target,
                ("box_vertex", "colour_fragment"),
                &view,
                &[wall],
            );
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            let pyramid = target.pyramid(&gpu, &culler.kernels, &mut encoder);
            let input = view_input(camera, pyramid.mip_count());
            let uniform = CullViewUniform::new(
                &input,
                CullPhase::Late,
                culler.slots,
                culler.storage.capacity,
                culler.kernels.workgroup,
            );
            culler
                .storage
                .write_uniform(&gpu.queue, CullPhase::Late, &uniform);
            let group = culler.kernels.transparent_bind_group(
                &gpu.device,
                &culler.storage.uniforms[CullPhase::Late as usize],
                &culler.storage,
                &pyramid,
                &mask,
            );
            culler
                .kernels
                .encode_transparent(&mut encoder, &group, &mask, None);
            gpu.queue.submit([encoder.finish()]);
            let words = read_texture(&gpu, &mask.texture, 0);
            let words = bytemuck::cast_slice::<u8, u32>(&words);
            let passed = passing_boxes(&gpu, &target, &view, &boxes);
            let levels = pyramid_levels(&gpu, &pyramid);
            for (slot, record) in records.iter().enumerate() {
                total += 1;
                let bit = bit(words, slot);
                assert!(
                    !(bit && passed.contains(&slot)),
                    "slot {slot} passes a depth-tested sample but was hidden"
                );
                let visible_lean = reference_transparent_occluded(record, &uniform, &levels, 1.0);
                let hidden_lean = reference_transparent_occluded(record, &uniform, &levels, -1.0);
                if visible_lean == hidden_lean {
                    decided += 1;
                    assert_eq!(bit, visible_lean, "slot {slot}");
                }
                if record.transparent_bounds().is_none() || slot == containing {
                    assert!(!bit, "slot {slot} has nothing to hide or holds the eye");
                }
                hidden += usize::from(bit);
                passing += usize::from(passed.contains(&slot));
            }
            assert!(
                (records.len()..words.len() * 32).all(|slot| !bit(words, slot)),
                "words past the records must read visible"
            );
        }
    }
    assert!(decided * 100 >= total * 98, "{decided} of {total} decided");
    assert!(hidden > 0, "the wall must hide some transparent bounds");
    assert!(passing > 0, "some transparent bounds must show");
}
