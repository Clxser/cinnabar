// Iris vertex stream writer shared by the cube, model and liquid expansion kernels. Each job
// writes one or more four-vertex faces; `geometry/vertex.rs` owns the word layout and generates
// the IRIS_WORD_* constants, `geometry/jobs.rs` the job encoding, `atlas.rs` the sprite slots.
import render::material::{MaterialGpu, texture_layer, texture_page, material_uses_overlay_mask};

struct ExpandParams {
    /// Block position that written vertex positions are relative to.
    anchor: vec4<i32>,
    /// x: AO goes to colour alpha (Iris `separateAo`); y: bake face shading (`oldLighting`);
    /// z: first job word in `job_words`; w: materials in the attribute table before it.
    flags: vec4<u32>,
}

// The material attribute table (two words per material) followed by four-word jobs; one
// buffer keeps every kernel within the baseline storage-buffer count per stage.
@group(1) @binding(0) var<storage, read> job_words: array<u32>;
@group(1) @binding(1) var<storage, read_write> vertices: array<u32>;
@group(1) @binding(2) var<storage, read> atlas_remap: array<vec2<u32>>;
@group(1) @binding(3) var<uniform> params: ExpandParams;

const EXPAND_WORKGROUP_SIZE: u32 = 64u;
const NO_ANIMATION: u32 = 0xffffffffu;
/// Iris's `mc_Entity.y` (`ExtendedDataHelper`): fluids and every other block.
const FLUID_RENDER_TYPE: f32 = 1.0;
const BLOCK_RENDER_TYPE: f32 = -1.0;

/// The four corners of one face, in counter-clockwise front-face order.
struct FaceCorners {
    /// Section-local positions.
    positions: array<vec3<f32>, 4>,
    /// Sprite-local texture coordinates in [0, 1].
    uvs: array<vec2<f32>, 4>,
    /// Block and sky light levels, 0 to 15.
    light: array<vec2<f32>, 4>,
    /// Ambient-occlusion brightness, 1 when unoccluded.
    ao: array<f32, 4>,
}

/// What every face of one job shares.
struct FaceSource {
    normal: vec3<f32>,
    /// Gamma-space vertex tint; alpha is the vertex opacity.
    tint: vec4<f32>,
    /// Vanilla directional face coefficient, applied when `params.flags.y` is set.
    face_shade: f32,
    /// Section-local corner of the block the face belongs to.
    block: vec3<f32>,
    chunk_origin: vec3<i32>,
    /// Material after positional variation; selects the sprite and block attributes.
    material_id: u32,
    material: MaterialGpu,
    two_sided: bool,
    /// Liquid surfaces: Iris's fluid render type in `mc_Entity.y`. Kernels leave it at the
    /// zero value, false, for blocks.
    fluid: bool,
}

/// Linear job index of a dispatch that may span a second dimension.
fn expand_job_index(id: vec3<u32>, groups: vec3<u32>) -> u32 {
    return id.x + id.y * groups.x * EXPAND_WORKGROUP_SIZE;
}

fn expand_job_count() -> u32 {
    return (arrayLength(&job_words) - params.flags.z) / 4u;
}

fn expand_job(index: u32) -> vec4<u32> {
    let word = params.flags.z + index * 4u;
    return vec4(job_words[word], job_words[word + 1u], job_words[word + 2u], job_words[word + 3u]);
}

/// Remap slot of a material's sprite; `opaque` selects the alpha-free overlay base.
fn sprite_slot(material: MaterialGpu, opaque: bool) -> u32 {
    var slot = ATLAS_HEADER_SLOTS;
    if (material.animation != NO_ANIMATION) {
        slot = ATLAS_ANIMATION_SLOT_BASE + material.animation;
    } else {
        let base = select(ATLAS_STATIC_SLOT_BASE, ATLAS_OPAQUE_SLOT_BASE, opaque);
        slot = base + texture_page(material.texture) * ATLAS_PAGE_SLOTS + u32(texture_layer(material.texture));
    }
    return select(ATLAS_HEADER_SLOTS, slot, slot < arrayLength(&atlas_remap));
}

/// Atlas texture coordinates of sprite-local `local` in remap slot `slot`.
fn atlas_uv(slot: u32, local: vec2<f32>) -> vec2<f32> {
    let size = vec2<f32>(atlas_remap[0]);
    let rect = atlas_remap[slot];
    let origin = vec2<f32>(f32(rect.x & 0xffffu), f32(rect.x >> 16u));
    let extent = vec2<f32>(f32(rect.y & 0xffffu), f32(rect.y >> 16u));
    return (origin + clamp(local, vec2(0.0), vec2(1.0)) * extent) / max(size, vec2(1.0));
}

/// `(block id, emission)` of a material: the Java `block.properties` id (-1 when unmapped)
/// and the block's light emission 0 to 15.
fn block_attributes(material_id: u32) -> vec2<i32> {
    if (material_id >= params.flags.w) {
        return vec2(-1, 0);
    }
    return bitcast<vec2<i32>>(vec2(job_words[material_id * 2u], job_words[material_id * 2u + 1u]));
}

/// Tangent with handedness in `w`, from the face's position and texture gradients.
fn face_tangent(positions: array<vec3<f32>, 4>, uvs: array<vec2<f32>, 4>, normal: vec3<f32>) -> vec4<f32> {
    let edge_a = positions[1] - positions[0];
    let edge_b = positions[2] - positions[0];
    let delta_a = uvs[1] - uvs[0];
    let delta_b = uvs[2] - uvs[0];
    let determinant = delta_a.x * delta_b.y - delta_b.x * delta_a.y;
    if (abs(determinant) < 1e-8) {
        let reference = select(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), abs(normal.x) > 0.9);
        return vec4(normalize(cross(reference, normal)), 1.0);
    }
    let tangent = (edge_a * delta_b.y - edge_b * delta_a.y) / determinant;
    let bitangent = (edge_b * delta_a.x - edge_a * delta_b.x) / determinant;
    let orthogonal = tangent - normal * dot(normal, tangent);
    let length_squared = dot(orthogonal, orthogonal);
    let direction = select(normalize(edge_a), orthogonal * inverseSqrt(length_squared), length_squared > 1e-12);
    return vec4(direction, select(1.0, -1.0, dot(cross(normal, direction), bitangent) < 0.0));
}

fn write_vertex(
    index: u32,
    position: vec3<f32>,
    colour: vec4<f32>,
    uv: vec2<f32>,
    mid_uv: vec2<f32>,
    light: vec2<f32>,
    normal: vec3<f32>,
    tangent: vec4<f32>,
    block_id: i32,
    render_type: f32,
    mid_block: vec4<f32>,
) {
    let base = index * IRIS_VERTEX_WORDS;
    let raw_light = clamp(light, vec2(0.0), vec2(15.0)) * 16.0;
    vertices[base + IRIS_WORD_POSITION] = bitcast<u32>(position.x);
    vertices[base + IRIS_WORD_POSITION + 1u] = bitcast<u32>(position.y);
    vertices[base + IRIS_WORD_POSITION + 2u] = bitcast<u32>(position.z);
    vertices[base + IRIS_WORD_COLOR] = pack4x8unorm(colour);
    vertices[base + IRIS_WORD_UV0] = pack2x16unorm(uv);
    vertices[base + IRIS_WORD_MID_TEX] = pack2x16unorm(mid_uv);
    vertices[base + IRIS_WORD_LIGHTMAP] = pack2x16float(raw_light);
    vertices[base + IRIS_WORD_NORMAL] = pack4x8snorm(vec4(normal, 0.0));
    vertices[base + IRIS_WORD_TANGENT] = pack4x8snorm(tangent);
    vertices[base + IRIS_WORD_BLOCK_ID] = bitcast<u32>(f32(block_id));
    vertices[base + IRIS_WORD_BLOCK_ID + 1u] = bitcast<u32>(render_type);
    vertices[base + IRIS_WORD_MID_BLOCK] = pack2x16float(mid_block.xy);
    vertices[base + IRIS_WORD_MID_BLOCK + 1u] = pack2x16float(mid_block.zw);
}

/// Writes face slot `face` as four coincident vertices, which rasterize nothing.
fn write_degenerate_face(face: u32, position: vec3<f32>) {
    for (var corner = 0u; corner < 4u; corner += 1u) {
        write_vertex(face * 4u + corner, position, vec4(0.0), vec2(0.0), vec2(0.0), vec2(0.0),
            vec3(0.0), vec4(0.0), -1, BLOCK_RENDER_TYPE, vec4(0.0));
    }
}

/// Writes one face with `tint` and the sprite in remap slot `slot`; `reversed` writes the back
/// side of a two-sided face, with its corners in the opposite order.
fn write_face(face: u32, source_corners: FaceCorners, source: FaceSource, tint: vec4<f32>, slot: u32, reversed: bool) {
    var corners = source_corners;
    var order = array<u32, 4>(0u, 1u, 2u, 3u);
    if (reversed) {
        order = array<u32, 4>(0u, 3u, 2u, 1u);
    }
    let normal = select(source.normal, -source.normal, reversed);
    let offset = vec3<f32>(source.chunk_origin - params.anchor.xyz);
    var positions: array<vec3<f32>, 4>;
    var local_uvs: array<vec2<f32>, 4>;
    var uvs: array<vec2<f32>, 4>;
    var mid_uv = vec2(0.0);
    for (var corner = 0u; corner < 4u; corner += 1u) {
        positions[corner] = corners.positions[order[corner]];
        local_uvs[corner] = corners.uvs[order[corner]];
        uvs[corner] = atlas_uv(slot, local_uvs[corner]);
        mid_uv += uvs[corner] * 0.25;
    }
    let tangent = face_tangent(positions, local_uvs, normal);
    let attributes = block_attributes(source.material_id);
    let separate_ao = params.flags.x != 0u;
    let shade = select(1.0, source.face_shade, params.flags.y != 0u);
    let centre = source.block + vec3(0.5);
    for (var corner = 0u; corner < 4u; corner += 1u) {
        let ao = corners.ao[order[corner]];
        let colour = vec4(
            tint.rgb * shade * select(ao, 1.0, separate_ao),
            select(tint.a, ao, separate_ao),
        );
        write_vertex(face * 4u + corner, offset + positions[corner], colour, uvs[corner], mid_uv,
            corners.light[order[corner]], normal, tangent, attributes.x,
            select(BLOCK_RENDER_TYPE, FLUID_RENDER_TYPE, source.fluid),
            vec4((centre - positions[corner]) * 64.0, f32(attributes.y)));
    }
}

/// Writes every face slot `job` reserved: the face, then an overlay slot when `JOB_OVERLAY_FACE`
/// is set and a back side when `JOB_REVERSE_FACE` is set. Grass-side overlay materials draw an
/// untinted opaque base under a tinted cutout overlay, as Java's overlay quads do; slots the
/// material does not need stay degenerate.
fn write_job_faces(job: vec4<u32>, corners: FaceCorners, source: FaceSource) {
    var face = job.z;
    let overlay = material_uses_overlay_mask(source.material.flags) && (job.w & JOB_OVERLAY_FACE) != 0u;
    if (overlay) {
        write_face(face, corners, source, vec4(vec3(1.0), source.tint.a), sprite_slot(source.material, true), false);
        write_face(face + 1u, corners, source, source.tint, sprite_slot(source.material, false), false);
    } else {
        write_face(face, corners, source, source.tint, sprite_slot(source.material, false), false);
        if ((job.w & JOB_OVERLAY_FACE) != 0u) {
            write_degenerate_face(face + 1u, vec3<f32>(source.chunk_origin - params.anchor.xyz) + corners.positions[0]);
        }
    }
    face += select(1u, 2u, (job.w & JOB_OVERLAY_FACE) != 0u);
    if ((job.w & JOB_REVERSE_FACE) != 0u) {
        if (source.two_sided) {
            write_face(face, corners, source, source.tint, sprite_slot(source.material, false), true);
        } else {
            write_degenerate_face(face, vec3<f32>(source.chunk_origin - params.anchor.xyz) + corners.positions[0]);
        }
    }
}

/// Writes degenerate faces into every slot of a job whose record failed validation.
fn write_invalid_job(job: vec4<u32>) {
    let slots = 1u + select(0u, 1u, (job.w & JOB_OVERLAY_FACE) != 0u) + select(0u, 1u, (job.w & JOB_REVERSE_FACE) != 0u);
    for (var slot = 0u; slot < slots; slot += 1u) {
        write_degenerate_face(job.z + slot, vec3(0.0));
    }
}
