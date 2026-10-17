//! Expansion kernels against an original packed section, and a translated fixture program
//! drawing the expanded faces.

use super::jobs::{JobKind, MaterialTables, SectionGeometry, plan_section};
use super::pipelines::{
    EXPAND_PARAMS_BYTES, expansion_shader_source, output_layout, source_layout,
};
use super::vertex::{
    BLOCK_ID, COLOR, CONSTANT_STREAM_WORDS, IRIS_VERTEX_WORDS, LIGHTMAP_FLOAT, LIGHTMAP_INT,
    MID_BLOCK, MID_TEX, NORMAL, POSITION, TANGENT, UV0,
};
use super::{LIGHTMAP_TEXTURE_MATRIX, vertex_buffer_layouts};
use crate::gpu_snapshot::Gpu;
use crate::shaderpack::atlas;
use assets::Material;
use bevy::math::{Mat4, Vec3};
use meshing::{Face, PackedQuad};
use wgpu::util::DeviceExt;

const COMMON_MODULE: &str = "render::shaderpack::geometry::expand_common";

fn kernel(root: &str) -> String {
    crate::shader_source::composed_prepared_with(
        &expansion_shader_source(root),
        &[],
        vec![(
            COMMON_MODULE,
            expansion_shader_source(include_str!("expand_common.wesl")),
        )],
    )
}

#[test]
fn every_expansion_kernel_composes_from_the_packed_decoders_and_validates() {
    for root in [
        include_str!("expand_cubes.wesl"),
        include_str!("expand_models.wesl"),
        include_str!("expand_liquids.wesl"),
    ] {
        let source = kernel(root);
        assert!(
            !source.contains("@vertex"),
            "imported draw entry points are pruned"
        );
    }
}

#[test]
fn production_devices_bind_every_kernels_storage_buffers() {
    // The client raises the device limit to what the packed chunk layout needs; the kernels
    // must fit within it.
    for kind in super::jobs::JOB_KINDS {
        assert!(
            super::pipelines::storage_buffers_per_stage(kind)
                <= crate::required_vertex_storage_buffers(),
            "{kind:?}"
        );
    }
}

// --- Fixture section --------------------------------------------------------------------------

const SECTION_ORIGIN: [i32; 3] = [32, 64, -16];
const ANCHOR: [i32; 3] = [16, 64, -16];
const ATLAS_SIZE: [u32; 2] = [256, 128];
/// Sprite of page 0 layer 3 in the fixture atlas: x, y, width, height.
const GRASS_RECT: [u32; 4] = [32, 16, 16, 16];
/// Sprite of page 1 layer 2.
const LEAF_RECT: [u32; 4] = [64, 0, 32, 32];
const GRASS: u32 = 1;
const LEAF: u32 = 2;
const GRASS_BLOCK_ID: i32 = 42;
const GRASS_EMISSION: u32 = 7;
const SPRITE_RGBA: [u8; 4] = [200, 100, 50, 255];

/// Light samples: block in bits 0-3, sky in 4-7, AO level in 8-10.
const fn light(block: u16, sky: u16, ao: u16) -> u16 {
    block | (sky << 4) | (ao << 8)
}

const TOP_LIGHT: [u16; 4] = [
    light(3, 15, 0),
    light(5, 15, 2),
    light(7, 14, 0),
    light(1, 13, 1),
];
const SIDE_LIGHT: [u16; 4] = [light(15, 0, 0); 4];

fn materials() -> Vec<Material> {
    let material = |texture: u32, flags| Material {
        texture: assets::TextureRef::from_raw(texture).unwrap(),
        flags,
        ..Material::unvaried()
    };
    vec![
        Material::unvaried(),
        material(3, 0),
        material((1 << 28) | 2, assets::MATERIAL_FLAG_TWO_SIDED),
    ]
}

fn cube_quads() -> [PackedQuad; 2] {
    [
        // A greedy 2x1 top face over blocks (2,5,3) and (3,5,3).
        PackedQuad::new([2, 5, 3], Face::PositiveY, 2, 1, GRASS),
        // A two-sided west face, outside the solid run.
        PackedQuad::new([6, 2, 1], Face::NegativeX, 1, 1, LEAF),
    ]
}

struct Expanded {
    gpu: Gpu,
    vertices: wgpu::Buffer,
    words: Vec<u32>,
    faces: u32,
    categories: [std::ops::Range<u32>; 3],
}

fn words_of(samples: [u16; 4]) -> [u32; 2] {
    [
        u32::from(samples[0]) | (u32::from(samples[1]) << 16),
        u32::from(samples[2]) | (u32::from(samples[3]) << 16),
    ]
}

fn read_buffer(gpu: &Gpu, buffer: &wgpu::Buffer, bytes: u64) -> Vec<u8> {
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &readback, 0, bytes);
    gpu.queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    rx.recv().unwrap().unwrap();
    readback
        .slice(..)
        .get_mapped_range()
        .expect("readback buffer is mapped")
        .to_vec()
}

fn layout(
    gpu: &Gpu,
    descriptor: &bevy::render::render_resource::BindGroupLayoutDescriptor,
) -> wgpu::BindGroupLayout {
    gpu.device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &descriptor.entries,
        })
}

/// Expands the fixture section's cube quads with the production kernel and layouts.
fn expand_fixture(name: &str) -> Option<Expanded> {
    let limits = wgpu::Limits {
        max_storage_buffers_per_shader_stage: super::pipelines::storage_buffers_per_stage(
            JobKind::Cube,
        ),
        ..wgpu::Limits::default()
    };
    let gpu = Gpu::for_fixture_with_limits(name, wgpu::Features::empty(), limits)?;
    let materials = materials();
    let quads = cube_quads();
    let tables = MaterialTables {
        materials: &materials,
        model_templates: &[],
        model_quads: &[],
    };
    let section = SectionGeometry {
        metadata_index: 0,
        quad_base: 0,
        cube_quads: &quads,
        solid_len: 1,
        model_refs: &[],
        model_draw_base: 0,
        model_draw_refs: &[],
        transparent_model_draw_base: 0,
        transparent_model_draw_refs: &[],
        liquid_base: 0,
        liquid_quads: &[],
    };
    let plan = plan_section(&section, &tables);
    let storage = |words: &[u32]| {
        gpu.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(words),
                usage: wgpu::BufferUsages::STORAGE,
            })
    };
    let quad_words: Vec<u32> = quads.iter().flat_map(PackedQuad::words).collect();
    let mut origin_words: Vec<u32> = SECTION_ORIGIN.map(|axis| axis as u32).to_vec();
    origin_words.extend([0, 0, 0, 0, 0]);
    let material_words: Vec<u32> = materials
        .iter()
        .flat_map(|material| {
            [
                material.texture.raw(),
                material.flags,
                material.animation,
                material.variation_start,
                material.variation_count,
                material.variation_weight,
            ]
        })
        .collect();
    let mut stream_words = words_of(TOP_LIGHT).to_vec();
    stream_words.extend(words_of(SIDE_LIGHT));
    let world_uniform = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&crate::material_shader::world_uniform_words(&[0; 4])),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let mut attributes: Vec<u32> = [(-1_i32) as u32, 0].repeat(materials.len());
    attributes[GRASS as usize * 2] = GRASS_BLOCK_ID as u32;
    attributes[GRASS as usize * 2 + 1] = GRASS_EMISSION;
    let mut job_words = attributes.clone();
    job_words.extend(plan.jobs[JobKind::Cube as usize].iter().flatten());
    let mut remap = vec![[0_u32; 2]; atlas::static_sprite_slot(1, 2) as usize + 1];
    remap[0] = ATLAS_SIZE;
    let rect = |[x, y, w, h]: [u32; 4]| [x | (y << 16), w | (h << 16)];
    remap[atlas::static_sprite_slot(0, 3) as usize] = rect(GRASS_RECT);
    remap[atlas::static_sprite_slot(1, 2) as usize] = rect(LEAF_RECT);
    let remap_words: Vec<u32> = remap.into_iter().flatten().collect();
    let mut params = [0_u32; (EXPAND_PARAMS_BYTES / 4) as usize];
    params[..3].copy_from_slice(&ANCHOR.map(|axis| axis as u32));
    params[6] = attributes.len() as u32;
    params[7] = materials.len() as u32;
    let params = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let vertex_bytes = u64::from(plan.faces) * 4 * u64::from(IRIS_VERTEX_WORDS) * 4;
    let vertices = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: vertex_bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::VERTEX
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let source = kernel(include_str!("expand_cubes.wesl"));
    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    let sources_layout = layout(&gpu, &source_layout(JobKind::Cube));
    let output_layout = layout(&gpu, &output_layout());
    let pipeline_layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&sources_layout), Some(&output_layout)],
            immediate_size: 0,
        });
    let pipeline = gpu
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("expand_cubes"),
            compilation_options: Default::default(),
            cache: None,
        });
    let source_buffers = [
        (1, storage(&quad_words)),
        (2, storage(&origin_words)),
        (3, storage(&material_words)),
        (7, storage(&[0; 64])),
        (8, storage(&[0; 64])),
        (13, storage(&stream_words)),
    ];
    let mut source_entries: Vec<_> = source_buffers
        .iter()
        .map(|(binding, buffer)| wgpu::BindGroupEntry {
            binding: *binding,
            resource: buffer.as_entire_binding(),
        })
        .collect();
    source_entries.push(wgpu::BindGroupEntry {
        binding: crate::material_shader::BIOME_QUERY_TABLES_BINDING,
        resource: world_uniform.as_entire_binding(),
    });
    let sources = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &sources_layout,
        entries: &source_entries,
    });
    let jobs = storage(&job_words);
    let remap = storage(&remap_words);
    let output = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &output_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: jobs.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: vertices.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: remap.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: params.as_entire_binding(),
            },
        ],
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &sources, &[]);
        pass.set_bind_group(1, &output, &[]);
        pass.dispatch_workgroups(
            (plan.jobs[JobKind::Cube as usize].len() as u32).div_ceil(64),
            1,
            1,
        );
    }
    gpu.queue.submit([encoder.finish()]);
    let bytes = read_buffer(&gpu, &vertices, vertex_bytes);
    Some(Expanded {
        gpu,
        vertices,
        words: bytemuck::cast_slice(&bytes).to_vec(),
        faces: plan.faces,
        categories: plan.categories,
    })
}

/// One decoded expanded vertex.
#[derive(Debug)]
struct Decoded {
    position: Vec3,
    colour: [u8; 4],
    uv: [f32; 2],
    mid_uv: [f32; 2],
    lightmap: [f32; 2],
    lightmap_int: [i16; 2],
    normal: [i8; 4],
    tangent: [i8; 4],
    block_id: f32,
    mid_block: [f32; 4],
}

fn decode(words: &[u32], vertex: u32) -> Decoded {
    let base = (vertex * IRIS_VERTEX_WORDS) as usize;
    let word = |field: super::vertex::StoredField, offset: u32| {
        words[base + (field.word + offset) as usize]
    };
    let unorm16 = |w: u32| [(w & 0xffff) as f32 / 65535.0, (w >> 16) as f32 / 65535.0];
    let half = |w: u32| [f16_to_f32(w & 0xffff), f16_to_f32(w >> 16)];
    let bytes = |w: u32| w.to_le_bytes();
    let signed = |w: u32| bytes(w).map(|b| b as i8);
    let mid_block = [half(word(MID_BLOCK, 0)), half(word(MID_BLOCK, 1))];
    Decoded {
        position: Vec3::new(
            f32::from_bits(word(POSITION, 0)),
            f32::from_bits(word(POSITION, 1)),
            f32::from_bits(word(POSITION, 2)),
        ),
        colour: bytes(word(COLOR, 0)),
        uv: unorm16(word(UV0, 0)),
        mid_uv: unorm16(word(MID_TEX, 0)),
        lightmap: half(word(LIGHTMAP_FLOAT, 0)),
        lightmap_int: [
            (word(LIGHTMAP_INT, 0) & 0xffff) as i16,
            (word(LIGHTMAP_INT, 0) >> 16) as i16,
        ],
        normal: signed(word(NORMAL, 0)),
        tangent: signed(word(TANGENT, 0)),
        block_id: f32::from_bits(word(BLOCK_ID, 0)),
        mid_block: [
            mid_block[0][0],
            mid_block[0][1],
            mid_block[1][0],
            mid_block[1][1],
        ],
    }
}

/// IEEE half-precision bits to `f32`.
fn f16_to_f32(bits: u32) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = ((bits >> 10) & 0x1f) as i32;
    let mantissa = (bits & 0x3ff) as f32;
    match exponent {
        0 => sign * mantissa * 2f32.powi(-24),
        31 => sign * f32::INFINITY,
        _ => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15),
    }
}

fn atlas_uv(rect: [u32; 4], local: [f32; 2]) -> [f32; 2] {
    [
        (rect[0] as f32 + local[0] * rect[2] as f32) / ATLAS_SIZE[0] as f32,
        (rect[1] as f32 + local[1] * rect[3] as f32) / ATLAS_SIZE[1] as f32,
    ]
}

fn assert_close(actual: &[f32], expected: &[f32], tolerance: f32, what: &str) {
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(a, e)| (a - e).abs() <= tolerance),
        "{what}: {actual:?} != {expected:?}"
    );
}

#[test]
fn greedy_top_face_expands_to_per_block_faces_with_java_attributes() {
    let Some(expanded) = expand_fixture("shader-pack cube expansion") else {
        return;
    };
    // Solid: two blocks of the greedy quad; cutout: the two-sided face and its back.
    assert_eq!(expanded.categories, [0..2, 2..4, 4..4]);
    assert_eq!(expanded.faces, 4);
    let offset = Vec3::new(16.0, 0.0, 0.0);
    let top_positions = [
        [
            Vec3::new(2.0, 6.0, 3.0),
            Vec3::new(2.0, 6.0, 4.0),
            Vec3::new(3.0, 6.0, 4.0),
            Vec3::new(3.0, 6.0, 3.0),
        ],
        [
            Vec3::new(3.0, 6.0, 3.0),
            Vec3::new(3.0, 6.0, 4.0),
            Vec3::new(4.0, 6.0, 4.0),
            Vec3::new(4.0, 6.0, 3.0),
        ],
    ];
    let locals = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
    // Light at each block corner, bilinear between the greedy quad's four samples:
    // (block, sky) raw Java coordinates and AO brightness.
    let lightmaps = [
        [[48.0, 240.0], [80.0, 240.0], [96.0, 232.0], [32.0, 224.0]],
        [[32.0, 224.0], [96.0, 232.0], [112.0, 224.0], [16.0, 208.0]],
    ];
    let ao = [[1.0, 0.6, 0.8, 0.9], [0.9, 0.8, 1.0, 0.8]];
    let mid_uv = atlas_uv(GRASS_RECT, [0.5, 0.5]);
    for face in 0..2 {
        for corner in 0..4 {
            let vertex = decode(&expanded.words, face * 4 + corner);
            let at = top_positions[face as usize][corner as usize];
            assert_close(
                &vertex.position.to_array(),
                &(at + offset).to_array(),
                1e-6,
                "position",
            );
            assert_close(
                &vertex.uv,
                &atlas_uv(GRASS_RECT, locals[corner as usize]),
                1e-4,
                "uv",
            );
            assert_close(&vertex.mid_uv, &mid_uv, 1e-4, "mc_midTexCoord");
            assert_close(
                &vertex.lightmap,
                &lightmaps[face as usize][corner as usize],
                0.01,
                "lightmap",
            );
            assert_eq!(
                vertex.lightmap_int,
                lightmaps[face as usize][corner as usize].map(|value| value.round() as i16)
            );
            let brightness = (ao[face as usize][corner as usize] * 255.0_f32).round() as u8;
            assert!(
                vertex.colour[..3]
                    .iter()
                    .all(|channel| channel.abs_diff(brightness) <= 1)
                    && vertex.colour[3] == 255,
                "colour carries AO: {:?} vs {brightness}",
                vertex.colour
            );
            assert_eq!(vertex.normal, [0, 127, 0, 0]);
            assert_eq!(
                vertex.tangent,
                [127, 0, 0, -127],
                "+X tangent, mirrored bitangent"
            );
            assert_eq!(vertex.block_id, GRASS_BLOCK_ID as f32);
            let centre = top_positions[face as usize][0] + Vec3::new(0.5, -0.5, 0.5);
            let expected = (centre - at) * 64.0;
            assert_close(
                &vertex.mid_block,
                &[expected.x, expected.y, expected.z, GRASS_EMISSION as f32],
                1e-3,
                "at_midBlock",
            );
        }
    }
}

#[test]
fn two_sided_faces_gain_an_explicit_back_face() {
    let Some(expanded) = expand_fixture("shader-pack two-sided expansion") else {
        return;
    };
    let front: Vec<_> = (8..12)
        .map(|vertex| decode(&expanded.words, vertex))
        .collect();
    let back: Vec<_> = (12..16)
        .map(|vertex| decode(&expanded.words, vertex))
        .collect();
    assert_eq!(front[0].normal, [-127, 0, 0, 0]);
    assert_eq!(back[0].normal, [127, 0, 0, 0]);
    let order = [0, 3, 2, 1];
    for (corner, from) in order.into_iter().enumerate() {
        assert_eq!(
            back[corner].position, front[from].position,
            "back face winds the other way"
        );
        assert_eq!(back[corner].uv, front[from].uv);
    }
    assert_eq!(
        front[0].block_id, -1.0,
        "materials without an id read Iris's -1"
    );
    assert_close(
        &front[0].uv,
        &atlas_uv(LEAF_RECT, [0.0, 1.0]),
        1e-4,
        "west face uv",
    );
    assert!(
        front[0].tangent[3] != back[0].tangent[3],
        "mirroring flips handedness"
    );
}

// --- Translated fixture program ----------------------------------------------------------------

const TARGET_SIDE: u32 = 64;

fn translate_fixture() -> shaderpack::ProgramTranslation {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/shaderpacks/terrain_probe");
    let pack = shaderpack::ShaderPack::open(&path).expect("fixture pack opens");
    let translation = pack.translate(&shaderpack::TranslateConfig::default());
    let program = translation
        .programs
        .into_iter()
        .find(|program| program.program.name == "gbuffers_terrain")
        .expect("fixture defines gbuffers_terrain");
    assert!(
        program.is_ok(),
        "{:?}",
        program.all_diagnostics().collect::<Vec<_>>()
    );
    program
}

fn uniform_words(block: &shaderpack::UniformBlock, model_view: Mat4, projection: Mat4) -> Vec<u8> {
    let mut bytes = vec![0_u8; block.size as usize];
    let mut put = |offset: u32, matrix: Mat4| {
        let start = offset as usize;
        bytes[start..start + 64].copy_from_slice(bytemuck::cast_slice(&matrix.to_cols_array()));
    };
    for member in &block.members {
        match member.name.as_str() {
            "iris_ModelViewMat" => put(member.offset, model_view),
            "iris_ProjMat" => put(member.offset, projection),
            "iris_TextureMat" => {
                let (count, stride) = member.array.unwrap_or((1, 64));
                for index in 0..count {
                    let matrix = if index == 1 {
                        LIGHTMAP_TEXTURE_MATRIX
                    } else {
                        Mat4::IDENTITY
                    };
                    put(member.offset + index * stride, matrix);
                }
            }
            _ => {}
        }
    }
    bytes
}

#[test]
fn translated_gbuffers_program_draws_expanded_terrain() {
    let Some(expanded) = expand_fixture("shader-pack terrain draw") else {
        return;
    };
    let program = translate_fixture();
    let reflection = program.reflection.as_ref().expect("reflection");
    let gpu = &expanded.gpu;
    let device = &gpu.device;
    let module = |stage: shaderpack::StageKind| {
        let compiled = program
            .stages
            .iter()
            .find(|translated| translated.stage == stage)
            .and_then(|translated| translated.result.as_ref().ok())
            .expect("stage compiled");
        let wgsl = naga::back::wgsl::write_string(
            &compiled.module,
            &compiled.info,
            naga::back::wgsl::WriterFlags::empty(),
        )
        .expect("stage writes as WGSL");
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        })
    };
    let entry = |stage| {
        reflection
            .entry_points
            .iter()
            .find(|(kind, _)| *kind == stage)
            .map(|(_, name)| name.clone())
            .expect("entry point")
    };
    let vertex_module = module(shaderpack::StageKind::Vertex);
    let fragment_module = module(shaderpack::StageKind::Fragment);

    // Group 0: the loose uniforms; group 1: the shared sampler and the atlas.
    let uniforms = reflection.uniforms.as_ref().expect("program uses uniforms");
    let eye = Vec3::new(19.0, 20.0, 3.5);
    let model_view =
        glam::camera::rh::view::look_at_mat4(eye, Vec3::new(19.0, 0.0, 3.5), Vec3::NEG_Z);
    let projection =
        glam::camera::rh::proj::directx::orthographic(-2.0, 2.0, -2.0, 2.0, 0.1, 100.0);
    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: &uniform_words(uniforms, model_view, projection),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let group0_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: uniforms.binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let gtexture = reflection
        .textures
        .iter()
        .find(|texture| texture.name == "gtexture")
        .expect("program samples gtexture");
    // The atlas's sampler class (repeat/nearest) has its own binding.
    let sampler_binding = reflection
        .samplers
        .iter()
        .find(|sampler| !sampler.comparison)
        .expect("program samples with a filtering sampler")
        .binding;
    let group1_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: sampler_binding,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: gtexture.binding,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    });
    let mut atlas_pixels = vec![0_u8; (ATLAS_SIZE[0] * ATLAS_SIZE[1] * 4) as usize];
    for y in GRASS_RECT[1]..GRASS_RECT[1] + GRASS_RECT[3] {
        for x in GRASS_RECT[0]..GRASS_RECT[0] + GRASS_RECT[2] {
            let texel = ((y * ATLAS_SIZE[0] + x) * 4) as usize;
            atlas_pixels[texel..texel + 4].copy_from_slice(&SPRITE_RGBA);
        }
    }
    let atlas = device.create_texture_with_data(
        &gpu.queue,
        &wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: ATLAS_SIZE[0],
                height: ATLAS_SIZE[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &atlas_pixels,
    );
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
    let atlas_view = atlas.create_view(&Default::default());
    let group0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &group0_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: uniforms.binding,
            resource: uniform_buffer.as_entire_binding(),
        }],
    });
    let group1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &group1_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: sampler_binding,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: gtexture.binding,
                resource: wgpu::BindingResource::TextureView(&atlas_view),
            },
        ],
    });

    let layouts =
        vertex_buffer_layouts(&reflection.vertex_inputs).expect("terrain supplies every input");
    let buffers: Vec<_> = layouts
        .iter()
        .map(|layout| {
            Some(wgpu::VertexBufferLayout {
                array_stride: layout.array_stride,
                step_mode: layout.step_mode,
                attributes: &layout.attributes,
            })
        })
        .collect();
    let targets: Vec<_> = reflection
        .color_outputs
        .iter()
        .map(|_| {
            Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })
        })
        .collect();
    assert_eq!(targets.len(), 2, "DRAWBUFFERS:01");
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&group0_layout), Some(&group1_layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &vertex_module,
            entry_point: Some(&entry(shaderpack::StageKind::Vertex)),
            compilation_options: Default::default(),
            buffers: &buffers,
        },
        // The translator flips clip-space Y, mirroring the counter-clockwise front faces.
        primitive: wgpu::PrimitiveState {
            front_face: wgpu::FrontFace::Cw,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &fragment_module,
            entry_point: Some(&entry(shaderpack::StageKind::Fragment)),
            compilation_options: Default::default(),
            targets: &targets,
        }),
        multiview_mask: None,
        cache: None,
    });

    let texture = |format| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: TARGET_SIDE,
                height: TARGET_SIDE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    };
    let colortex = [
        texture(wgpu::TextureFormat::Rgba8Unorm),
        texture(wgpu::TextureFormat::Rgba8Unorm),
    ];
    let depth = texture(wgpu::TextureFormat::Depth32Float);
    let views: Vec<_> = colortex
        .iter()
        .map(|texture| texture.create_view(&Default::default()))
        .collect();
    let depth_view = depth.create_view(&Default::default());
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&super::draw::face_indices(expanded.faces)),
        usage: wgpu::BufferUsages::INDEX,
    });
    let constants = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&CONSTANT_STREAM_WORDS),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let attachments: Vec<_> = views
            .iter()
            .map(|view| {
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })
            })
            .collect();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &attachments,
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group0, &[]);
        pass.set_bind_group(1, &group1, &[]);
        pass.set_vertex_buffer(super::GEOMETRY_VERTEX_BUFFER, expanded.vertices.slice(..));
        pass.set_vertex_buffer(super::CONSTANT_VERTEX_BUFFER, constants.slice(..));
        pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        let solid = &expanded.categories[0];
        pass.draw_indexed(
            0..(solid.end - solid.start) * 6,
            (solid.start * 4) as i32,
            0..1,
        );
    }
    gpu.queue.submit([encoder.finish()]);
    let pixels: Vec<_> = colortex
        .iter()
        .map(|texture| read_target(gpu, texture))
        .collect();
    let pixel = |target: usize, x: u32, y: u32| {
        let index = ((y * TARGET_SIDE + x) * 4) as usize;
        [
            pixels[target][index],
            pixels[target][index + 1],
            pixels[target][index + 2],
            pixels[target][index + 3],
        ]
    };
    // Block (2,5,3)'s top centre sits on its faces' shared diagonal, between AO 1.0 and 0.8.
    // Rows are stored bottom-up (the translator flips clip-space Y), so it lands on row 31.
    let albedo = pixel(0, 24, 31);
    let expected = SPRITE_RGBA.map(|channel| (f32::from(channel) * 0.9).round() as u8);
    assert!(
        albedo[..3]
            .iter()
            .zip(&expected)
            .all(|(a, e)| a.abs_diff(*e) <= 3)
            && albedo[3] == 255,
        "albedo {albedo:?} vs {expected:?}"
    );
    // Lightmap (48,240)..(96,232) midway, through gl_TextureMatrix[1]: (72 + 8) / 256, (236 + 8) / 256.
    let data = pixel(1, 24, 31);
    let lightmap = [80.0 / 256.0 * 255.0, 244.0 / 256.0 * 255.0];
    assert!(
        (f32::from(data[0]) - lightmap[0]).abs() <= 2.0
            && (f32::from(data[1]) - lightmap[1]).abs() <= 2.0,
        "lightmap {data:?} vs {lightmap:?}"
    );
    assert_eq!(
        data[2], GRASS_BLOCK_ID as u8,
        "mc_Entity.x reaches the program"
    );
    assert_eq!(pixel(0, 4, 4), [0; 4], "nothing draws outside the faces");
}

fn read_target(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u8> {
    let bytes = u64::from(TARGET_SIDE * TARGET_SIDE * 4);
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(TARGET_SIDE * 4),
                rows_per_image: Some(TARGET_SIDE),
            },
        },
        wgpu::Extent3d {
            width: TARGET_SIDE,
            height: TARGET_SIDE,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    rx.recv().unwrap().unwrap();
    readback
        .slice(..)
        .get_mapped_range()
        .expect("readback buffer is mapped")
        .to_vec()
}
