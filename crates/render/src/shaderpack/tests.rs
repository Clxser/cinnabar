//! Original fixture packs through planning and, with a GPU, through a whole frame.

use super::loader::{GpuContext, build, class_sampler, runnable_programs, translate_config};
use super::passes::{FrameIo, ViewState, record_frame};
use super::plan::plan_frame;
use super::uniforms::{FrameClock, FrameUniforms, ViewCamera};
use bevy::math::{Quat, Vec3};
use render_scene::shaderpack::ShaderPackFrameInputs;
use shaderpack::{JAVA_DIMENSIONS, PackSource, ProgramGroup, SamplerClass, ShaderPack};
use std::sync::Arc;

const PASS_VERTEX: &str = "#version 120
varying vec2 texcoord;
void main() {
    gl_Position = ftransform();
    texcoord = gl_MultiTexCoord0.xy;
}
";

/// Inverts `colortex0` through texture coordinates.
const INVERT: &str = "#version 120
uniform sampler2D colortex0;
varying vec2 texcoord;
void main() {
    gl_FragData[0] = vec4(1.0 - texture2D(colortex0, texcoord).rgb, 1.0);
}
";

/// Copies `colortex0` to the screen through window coordinates.
const FINAL: &str = "#version 120
uniform sampler2D colortex0;
uniform float viewWidth;
uniform float viewHeight;
void main() {
    gl_FragColor = texture2D(colortex0, gl_FragCoord.xy / vec2(viewWidth, viewHeight));
}
";

/// Writes `colortex3` and leaves `colortex0` alone.
const FILL: &str = "#version 120
void main() {
/* DRAWBUFFERS:3 */
    gl_FragData[0] = vec4(0.25);
}
";

fn pack(files: &[(&str, &str)]) -> ShaderPack {
    ShaderPack::new(PackSource::from_files(files.iter().copied()))
}

#[test]
fn enabled_full_screen_programs_run_in_frame_order() {
    let pack = pack(&[
        (
            "shaders.properties",
            "program.composite1.enabled = WANT_BLUR\nflip.composite2.colortex3 = false\n",
        ),
        ("composite.vsh", PASS_VERTEX),
        ("composite.fsh", INVERT),
        ("composite1.vsh", PASS_VERTEX),
        ("composite1.fsh", INVERT),
        ("composite2.vsh", PASS_VERTEX),
        ("composite2.fsh", FILL),
        ("deferred.vsh", PASS_VERTEX),
        ("deferred.fsh", INVERT),
        ("final.vsh", PASS_VERTEX),
        ("final.fsh", FINAL),
        ("gbuffers_terrain.vsh", PASS_VERTEX),
        ("gbuffers_terrain.fsh", FILL),
    ]);
    let translation = pack.translate(&translate_config(&pack, JAVA_DIMENSIONS[0]));
    let (runnable, plans) = runnable_programs(&translation).expect("fixture programs translate");
    let names: Vec<&str> = runnable.iter().map(|p| p.program.name.as_str()).collect();
    assert_eq!(names, ["deferred", "composite", "composite2", "final"]);
    let frame = plan_frame(&plans, &translation.properties.flips);
    let writes: Vec<_> = frame.post.iter().map(|s| s.writes.clone()).collect();
    assert_eq!(
        writes,
        [
            vec![Some((0, true))],
            vec![Some((0, false))],
            vec![Some((3, true))],
            vec![None],
        ]
    );
    assert!(
        frame.swap_back.is_empty(),
        "composite2's flip is suppressed"
    );
    assert_eq!(plans[3].group, ProgramGroup::Final);
}

#[test]
fn a_failed_required_program_fails_the_load() {
    let pack = pack(&[
        ("composite.vsh", PASS_VERTEX),
        (
            "composite.fsh",
            "#version 120\nvoid main() { gl_FragData[0] = undefined_name; }\n",
        ),
    ]);
    let translation = pack.translate(&translate_config(&pack, JAVA_DIMENSIONS[0]));
    let error = runnable_programs(&translation).expect_err("composite fails");
    assert!(error.contains("composite"), "{error}");
}

fn gpu() -> Option<GpuContext> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = bevy::tasks::block_on(instance.request_adapter(&Default::default())).ok()?;
    let (device, queue) = bevy::tasks::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("shader pack frame fixture"),
        required_features: adapter.features(),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .ok()?;
    Some(GpuContext {
        device,
        queue,
        adapter,
    })
}

/// Side of the fixture frame.
const SIZE: [u32; 2] = [4, 4];

/// Runs `files` over a scene whose top half is red and bottom half mid grey; returns the
/// output rows top-down, 256 bytes apart.
fn render(gpu: &GpuContext, files: &[(&str, &str)]) -> Vec<u8> {
    let pack = pack(files);
    let pack =
        Arc::new(build(&pack, "fixture", JAVA_DIMENSIONS[0], None, gpu).expect("fixture loads"));
    let size = SIZE;
    let extent = wgpu::Extent3d {
        width: size[0],
        height: size[1],
        depth_or_array_layers: 1,
    };
    let texture = |format, usage| {
        gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let format = render_scene::SCENE_COLOR_FORMAT;
    let scene = texture(
        format,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    // Top half red, bottom half mid grey, rows top-down as wgpu stores them.
    let pixels: Vec<u8> = (0..size[1])
        .flat_map(|row| {
            let colour = if row < 2 {
                [255, 0, 0, 255]
            } else {
                [128, 128, 128, 255]
            };
            std::iter::repeat_n(colour, size[0] as usize).flatten()
        })
        .collect();
    gpu.queue.write_texture(
        scene.as_image_copy(),
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * size[0]),
            rows_per_image: None,
        },
        extent,
    );
    let depth = texture(
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let output = texture(
        format,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let mut state = ViewState::new(&gpu.device, Arc::clone(&pack), size);
    let game = ShaderPackFrameInputs::default();
    let frame = FrameUniforms::new(
        ViewCamera {
            position: Vec3::new(0.0, 64.0, 0.0),
            rotation: Quat::IDENTITY,
            focal: [1.0, 1.0],
            screen: size,
        },
        &game,
        &pack.settings,
        FrameClock {
            elapsed_seconds: 1.0,
            delta_seconds: 0.016,
        },
        &mut state.history,
    );
    state.write_uniforms(&gpu.queue, &frame, 0.016, Vec3::ZERO);
    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &depth.create_view(&Default::default()),
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(0.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    drop(clear);
    let io = FrameIo {
        scene_colour: &scene.create_view(&Default::default()),
        scene_depth: &depth.create_view(&Default::default()),
        scene_near: 0.1,
        render_distance: game.render_distance,
        destination: &output.create_view(&Default::default()),
        destination_format: format,
        geometry: None,
    };
    record_frame(&gpu.device, &gpu.queue, &mut encoder, &mut state, &io);
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256 * u64::from(size[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: None,
            },
        },
        extent,
    );
    gpu.queue.submit([encoder.finish()]);
    if let Some(error) = bevy::tasks::block_on(scope.pop()) {
        panic!("validation error: {error}");
    }
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    readback.slice(..).get_mapped_range().unwrap().to_vec()
}

fn assert_pixels(bytes: &[u8], expected: impl Fn(usize) -> [u8; 3]) {
    for row in 0..SIZE[1] as usize {
        for column in 0..SIZE[0] as usize {
            let at = row * 256 + column * 4;
            let pixel = &bytes[at..at + 4];
            let expected = expected(row);
            for c in 0..3 {
                assert!(
                    pixel[c].abs_diff(expected[c]) <= 1,
                    "row {row} column {column}: {pixel:?} vs {expected:?}"
                );
            }
        }
    }
}

#[test]
fn composite_and_final_transform_the_scene_and_present_it_upright() {
    let Some(gpu) = gpu() else {
        eprintln!("missing fixture: native GPU adapter for the shader pack frame");
        return;
    };
    let bytes = render(
        &gpu,
        &[
            ("composite.vsh", PASS_VERTEX),
            ("composite.fsh", INVERT),
            ("final.vsh", PASS_VERTEX),
            ("final.fsh", FINAL),
        ],
    );
    assert_pixels(&bytes, |row| {
        if row < 2 {
            [0, 255, 255]
        } else {
            [127, 127, 127]
        }
    });
}

/// Reads `colortex0` exactly on its top edge, where a repeating sampler would blend in the
/// bottom row.
const TOP_EDGE: &str = "#version 120
uniform sampler2D colortex0;
void main() {
    gl_FragData[0] = texture2D(colortex0, vec2(0.5, 1.0));
}
";

#[test]
fn render_targets_clamp_at_their_edges() {
    let Some(gpu) = gpu() else {
        eprintln!("missing fixture: native GPU adapter for the shader pack frame");
        return;
    };
    let bytes = render(
        &gpu,
        &[
            ("composite.vsh", PASS_VERTEX),
            ("composite.fsh", TOP_EDGE),
            ("final.vsh", PASS_VERTEX),
            ("final.fsh", FINAL),
        ],
    );
    assert_pixels(&bytes, |_| [255, 0, 0]);
}

/// Multiplies a 32-bit float `colortex0` by `depthtex0` (1.0 at the cleared far plane).
const FLOAT_AND_DEPTH: &str = "#version 120
/*
const int colortex0Format = RGBA32F;
*/
uniform sampler2D colortex0;
uniform sampler2D depthtex0;
varying vec2 texcoord;
void main() {
    gl_FragData[0] = vec4(texture2D(colortex0, texcoord).rgb * texture2D(depthtex0, texcoord).r, 1.0);
}
";

#[test]
fn float32_targets_and_depth_bind_with_their_samplers() {
    let Some(gpu) = gpu() else {
        eprintln!("missing fixture: native GPU adapter for the shader pack frame");
        return;
    };
    let bytes = render(
        &gpu,
        &[
            ("composite.vsh", PASS_VERTEX),
            ("composite.fsh", FLOAT_AND_DEPTH),
            ("final.vsh", PASS_VERTEX),
            ("final.fsh", FINAL),
        ],
    );
    assert_pixels(&bytes, |row| {
        if row < 2 {
            [255, 0, 0]
        } else {
            [128, 128, 128]
        }
    });
}

/// Reads `colortex0` through a 3D sampler, where the pack's 3D custom texture binds.
const VOLUME: &str = "#version 120
uniform sampler3D colortex0;
void main() {
/* DRAWBUFFERS:3 */
    gl_FragData[0] = texture3D(colortex0, vec3(0.5));
}
";

#[test]
fn a_3d_custom_texture_leaves_2d_reads_of_its_name_on_the_buffer() {
    let Some(gpu) = gpu() else {
        eprintln!("missing fixture: native GPU adapter for the shader pack frame");
        return;
    };
    // As Photon does: `colortex0` names both the scene buffer (sampler2D) and a 3D noise
    // volume (sampler3D) in composite programs.
    let bytes = render(
        &gpu,
        &[
            (
                "shaders.properties",
                "texture.composite.colortex0 = volume.dat TEXTURE_3D R8 2 2 2 RED UNSIGNED_BYTE\n",
            ),
            ("volume.dat", "ABCDEFGH"),
            ("composite.vsh", PASS_VERTEX),
            ("composite.fsh", VOLUME),
            ("composite1.vsh", PASS_VERTEX),
            ("composite1.fsh", INVERT),
            ("final.vsh", PASS_VERTEX),
            ("final.fsh", FINAL),
        ],
    );
    assert_pixels(&bytes, |row| {
        if row < 2 {
            [0, 255, 255]
        } else {
            [127, 127, 127]
        }
    });
}

/// Inverts `colortex0` in place: samples it and writes `colorimg0`, the same texture.
const INVERT_IN_PLACE: &str = "#version 430
layout(local_size_x = 4, local_size_y = 4) in;
const ivec3 workGroups = ivec3(1, 1, 1);
uniform sampler2D colortex0;
layout(rgba8) uniform image2D colorimg0;
void main() {
    ivec2 p = ivec2(gl_GlobalInvocationID.xy);
    imageStore(colorimg0, p, vec4(1.0 - texelFetch(colortex0, p, 0).rgb, 1.0));
}
";

#[test]
fn a_buffer_sampled_and_written_as_an_image_reads_a_snapshot() {
    let Some(gpu) = gpu() else {
        eprintln!("missing fixture: native GPU adapter for the shader pack frame");
        return;
    };
    let bytes = render(
        &gpu,
        &[
            ("composite.csh", INVERT_IN_PLACE),
            ("final.vsh", PASS_VERTEX),
            ("final.fsh", FINAL),
        ],
    );
    assert_pixels(&bytes, |row| {
        if row < 2 {
            [0, 255, 255]
        } else {
            [127, 127, 127]
        }
    });
}

#[test]
fn sampler_classes_map_to_wrap_and_filter_modes() {
    for (index, class) in SamplerClass::ALL.into_iter().enumerate() {
        assert_eq!(
            class as usize, index,
            "PackGpu::filtering is indexed by class"
        );
        let d = class_sampler(class);
        let wrap = if class.clamps() {
            wgpu::AddressMode::ClampToEdge
        } else {
            wgpu::AddressMode::Repeat
        };
        assert_eq!(
            [d.address_mode_u, d.address_mode_v, d.address_mode_w],
            [wrap; 3]
        );
        let filter = if class.linear() {
            wgpu::FilterMode::Linear
        } else {
            wgpu::FilterMode::Nearest
        };
        assert_eq!([d.mag_filter, d.min_filter], [filter; 2]);
    }
}
