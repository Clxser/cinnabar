//! Actors, block entities and the first-person hand for shader packs: Cinnabar's actor rig
//! skinned into the Iris vertex stream and drawn with the pack's `gbuffers_entities`,
//! `gbuffers_entities_translucent`, `gbuffers_spidereyes`, `gbuffers_armor_glint`,
//! `gbuffers_block`, `gbuffers_block_translucent`, `gbuffers_beaconbeam`, `gbuffers_hand`,
//! `gbuffers_hand_water`, `shadow_entities` and `shadow_block` programs (each through Iris's
//! fallback chain, see [`super::draws::DrawKind`]).
//!
//! # Skinning
//! Each frame one compute pass ([`skin`], `skin.wesl`) poses every drawn rig instance with
//! the actor pass's own skinning (`render::actor_rig`) and writes one Iris vertex per rig
//! vertex: positions relative to the camera (the hand's in view space), world normals, the
//! instance's light as Java lightmap coordinates, its dye as the vertex colour, and an
//! explicit reversed back face for every two-sided triangle, so pack pipelines cull back
//! faces as Java's entity render types do. Block-entity vertex lists go through a second
//! entry point of the same kernel. Both stream into one per-view vertex buffer that every
//! actor draw (camera and shadow) reads; the geometry adapter's constant stream supplies
//! the attributes actors do not carry, as it does for terrain.
//!
//! # Draws
//! A rig instance is one draw per pass it appears in. Its texture layer is bound as
//! `gtexture` through a single-layer, non-sRGB view of Cinnabar's actor artwork (raw gamma
//! bytes, as Java textures read), with neutral LabPBR `normals`/`specular`. Iris's per-entity
//! uniforms are patched into the draw's own copy of the draw kind's uniform block:
//! `entityId` (the actor's Bedrock identifier through `entity.properties`), `entityColor`
//! (the hurt/creeper-flash overlay), `blockEntityId` and `currentRenderedItemId` (the held
//! item's `item.properties` id for first-person items). Glinted instances add a
//! `gbuffers_armor_glint` draw over the same vertices with the glint raster and Java's glint
//! texture matrix. Block entities use `mc_Entity`/`blockEntityId` -1: Cinnabar merges their
//! meshes without the source block state.
//!
//! # Not drawn by the pack
//! Entities with the glowing effect (`gbuffers_entities_glowing`; Cinnabar has no outline
//! state), lightning (`gbuffers_lightning`, no rig), block cracks, block-entity portals and
//! the selection outline stay in the Vanilla scene and reach the pack through the overlay.

mod ids;
mod skin;
mod sources;
#[cfg(test)]
mod tests;

pub(crate) use ids::{ActorIdMaps, id_tables};
pub(crate) use skin::{ActorSkinPipelines, load_shaders};
pub(crate) use sources::{
    ActorSources, BlockEntityList, BlockEntitySource, Hand, LayerTexture, RigBuffers, RigInstance,
    RigSource,
};

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use assets::{EntityRenderMaterial, EntityRenderMaterialState};
use bevy::math::{Mat4, Vec3, Vec4};
use bevy::render::render_resource::{Texture, TextureId};
use wgpu::{CommandEncoder, Device, Queue, TextureView};

use super::draws::DrawKind;
use super::frame_geometry::open_kind;
use super::geometry::{CONSTANT_VERTEX_BUFFER, GEOMETRY_VERTEX_BUFFER, IRIS_VERTEX_STRIDE};
use super::passes::ViewState;
use super::targets::{AtlasViews, draw_read_alt, pass_bind_groups};
use super::uniforms::{HostValue, patch_block};
use skin::{
    JOB_ONE_SIDED, JOB_VIEW_SPACE, SKIN_PARAMS_BYTES, SKIN_WORKGROUP_SIZE, SkinJob, SkinParams,
};

/// A texture layer's identity: the texture and the layer.
type TextureKey = (TextureId, u32);

/// Java's glint texture-matrix scale for worn and entity glint, and for held items.
const ENTITY_GLINT_SCALE: f32 = 0.16;
const ITEM_GLINT_SCALE: f32 = 8.0;
/// Java's default glint speed option.
const GLINT_SPEED: f64 = 0.5;
/// Compute dispatch dimension limit.
const MAX_WORKGROUPS: u32 = 65_535;

/// Java's glint texture matrix (`RenderStateShard.setupGlintTexturing`) at `seconds`.
pub(crate) fn glint_texture_matrix(seconds: f32, scale: f32) -> Mat4 {
    let ticks = (f64::from(seconds) * 1000.0 * GLINT_SPEED * 8.0) as i64;
    let u = (ticks % 110_000) as f32 / 110_000.0;
    let v = (ticks % 30_000) as f32 / 30_000.0;
    Mat4::from_translation(Vec3::new(-u, v, 0.0))
        * Mat4::from_rotation_z(10f32.to_radians())
        * Mat4::from_scale(Vec3::splat(scale))
}

/// The first-person hand's projection: Java's perspective at the hand's field of view with
/// its clip depth scaled by `MC_HAND_DEPTH`, which keeps hand pixels at window depths around
/// one half, where packs recognise them.
pub(crate) fn hand_projection(fov_radians: f32, screen: [u32; 2], far: f32) -> Mat4 {
    let focal_y = 1.0 / (fov_radians * 0.5).tan();
    let aspect = screen[0] as f32 / screen[1].max(1) as f32;
    let projection =
        super::uniforms::gl_perspective([focal_y / aspect, focal_y], super::uniforms::GL_NEAR, far);
    Mat4::from_scale(Vec3::new(1.0, 1.0, HAND_DEPTH)) * projection
}

/// Iris's `MC_HAND_DEPTH`.
pub(crate) const HAND_DEPTH: f32 = 0.125;

/// The per-draw values Iris sets around each entity, block entity and item.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DrawValues {
    pub entity_id: i32,
    pub block_entity_id: i32,
    pub item_id: i32,
    pub entity_color: Vec4,
    /// `gl_TextureMatrix[0]`: identity except for the glint's scroll.
    pub texture_matrix: Mat4,
}

impl DrawValues {
    pub(crate) const NONE: Self = Self {
        entity_id: -1,
        block_entity_id: -1,
        item_id: -1,
        entity_color: Vec4::ZERO,
        texture_matrix: Mat4::IDENTITY,
    };

    /// The uniform members these values patch.
    pub(crate) fn uniforms(&self) -> [(&'static str, HostValue); 6] {
        [
            ("entityId", HostValue::Int(self.entity_id)),
            ("blockEntityId", HostValue::Int(self.block_entity_id)),
            ("currentRenderedItemId", HostValue::Int(self.item_id)),
            ("entityColor", HostValue::Vec(self.entity_color, 4)),
            ("iris_TextureMat[0]", HostValue::Mat4(self.texture_matrix)),
            ("textureMatrix", HostValue::Mat4(self.texture_matrix)),
        ]
    }
}

/// How a rig instance draws: its camera draw kind and the work it needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RigClass {
    pub kind: DrawKind,
    /// Casts a shadow (`shadow_entities`).
    pub shadow: bool,
    /// Its material culls authored back faces.
    pub one_sided: bool,
    /// Draws the enchantment glint over itself.
    pub glint: bool,
}

/// The draw kind and state of a rig instance with `material` (an actor-pass material word),
/// or `None` for depth-only helpers. `blend` overrides the material's blending (held items);
/// `hand` selects the first-person kinds.
pub(crate) fn classify(
    material: u32,
    item_extent: f32,
    blend: Option<bool>,
    hand: bool,
) -> Option<RigClass> {
    let kind_bits = material & EntityRenderMaterialState::KIND_MASK;
    if kind_bits == EntityRenderMaterial::DissolveDepth as u32 {
        return None;
    }
    let state = crate::actor::material::state(material);
    let blended = blend.unwrap_or(state.is_some_and(|state| state.blend));
    let additive = state.is_some_and(|state| state.blend && state.additive);
    let kind = match (hand, additive, blended) {
        (true, _, true) => DrawKind::HandWater,
        (true, _, false) => DrawKind::Hand,
        (false, true, _) => DrawKind::SpiderEyes,
        (false, false, true) => DrawKind::EntitiesTranslucent,
        (false, false, false) => DrawKind::Entities,
    };
    Some(RigClass {
        kind,
        shadow: !hand && !additive,
        one_sided: state.is_some_and(|state| state.cull),
        glint: kind_bits == EntityRenderMaterial::Glint as u32 || item_extent != 0.0,
    })
}

/// A little-endian RGBA8 word as unit floats.
fn unpack_rgba8(word: u32) -> Vec4 {
    Vec4::from_array(word.to_le_bytes().map(|byte| f32::from(byte) / 255.0))
}

/// The vertex colour of an instance: its dye (`change_color`), white without one; blended
/// materials also take the dye's alpha, as the actor pass multiplies them by it.
pub(crate) fn vertex_colour(tint: u32, blended: bool) -> u32 {
    if tint == 0 {
        return u32::MAX;
    }
    if blended { tint } else { tint | 0xff00_0000 }
}

/// The pack's `entityColor` of an instance: its overlay, none where the material disables it.
pub(crate) fn entity_colour(overlay: u32, material: u32) -> Vec4 {
    if crate::actor::material::state(material).is_some_and(|state| state.disable_overlay) {
        return Vec4::ZERO;
    }
    unpack_rgba8(overlay)
}

/// One actor draw: its vertices in the view's stream, its texture and its values.
#[derive(Clone, Debug, PartialEq)]
struct ActorDraw {
    kind: DrawKind,
    vertices: Range<u32>,
    texture: TextureKey,
    values: DrawValues,
}

/// One draw's uniform copy and the bind groups over it and its texture.
struct Slot {
    uniforms: wgpu::Buffer,
    texture: Option<TextureKey>,
    groups: [Option<wgpu::BindGroup>; 4],
}

/// A storage buffer that grows to the largest size asked of it.
#[derive(Default)]
struct Growable {
    buffer: Option<wgpu::Buffer>,
}

impl Growable {
    fn ensure(
        &mut self,
        device: &Device,
        label: &str,
        size: u64,
        usage: wgpu::BufferUsages,
    ) -> &wgpu::Buffer {
        let size = size.max(16).next_multiple_of(16);
        if self.buffer.as_ref().is_none_or(|b| b.size() < size) {
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size.next_power_of_two(),
                usage,
                mapped_at_creation: false,
            }));
        }
        self.buffer.as_ref().expect("just ensured")
    }
}

/// One kernel dispatch of the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dispatch {
    /// Rig jobs `first..first + count` of the actors (`hand == false`) or the hand.
    Rig {
        hand: bool,
        first: u32,
        count: u32,
        max_vertices: u32,
    },
    /// Block-entity list `list` (0 solid, 1 overlay, 2 additive) into `first_output`.
    BlockEntities {
        list: usize,
        first_output: u32,
        count: u32,
    },
}

/// A view's actor geometry: skinned vertices, draws and their bindings.
#[derive(Default)]
pub(crate) struct ViewActors {
    draws: Vec<ActorDraw>,
    /// Each draw's slot within its kind, parallel to `draws`.
    slot_of: Vec<u32>,
    jobs: Vec<SkinJob>,
    dispatches: Vec<Dispatch>,
    slots: HashMap<DrawKind, Vec<Slot>>,
    views: HashMap<TextureKey, (TextureView, u64)>,
    output: Growable,
    job_buffer: Growable,
    params: Growable,
    neutral: Option<NeutralTextures>,
    frame: u64,
    scratch: Vec<u8>,
}

/// LabPBR's neutral `normals` and `specular`, bound for every actor texture.
struct NeutralTextures {
    normals: TextureView,
    specular: TextureView,
}

impl NeutralTextures {
    fn new(device: &Device, queue: &Queue) -> Self {
        let texel = |label: &str, rgba: [u8; 4]| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                texture.as_image_copy(),
                &rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                texture.size(),
            );
            texture.create_view(&Default::default())
        };
        Self {
            normals: texel("shader-pack actor normals", super::atlas::NEUTRAL_NORMAL),
            specular: texel("shader-pack actor specular", super::atlas::NEUTRAL_SPECULAR),
        }
    }
}

/// A single-layer 2D view of `texture`, in its non-sRGB format.
fn layer_view(texture: &Texture, layer: u32) -> TextureView {
    wgpu::Texture::create_view(
        texture,
        &wgpu::TextureViewDescriptor {
            label: Some("shader-pack actor texture layer"),
            format: Some(texture.format().remove_srgb_suffix()),
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_array_layer: layer,
            array_layer_count: Some(1),
            ..Default::default()
        },
    )
}

impl ViewActors {
    /// Drops this frame's work, keeping allocations.
    fn begin(&mut self) {
        self.draws.clear();
        self.slot_of.clear();
        self.jobs.clear();
        self.dispatches.clear();
        self.frame += 1;
    }

    /// The key of `texture`'s view, creating the view on first use.
    fn view(&mut self, texture: LayerTexture<'_>) -> TextureKey {
        let key = (texture.texture.id(), texture.layer);
        let frame = self.frame;
        self.views
            .entry(key)
            .and_modify(|(_, used)| *used = frame)
            .or_insert_with(|| (layer_view(texture.texture, texture.layer), frame));
        key
    }

    fn push(
        &mut self,
        kind: DrawKind,
        vertices: Range<u32>,
        texture: TextureKey,
        values: DrawValues,
    ) {
        self.draws.push(ActorDraw {
            kind,
            vertices,
            texture,
            values,
        });
    }

    /// Plans the jobs and draws of one rig source; `output` is the next free output vertex.
    fn plan_rig(
        &mut self,
        source: &dyn RigSource,
        hand: bool,
        sources: &ActorSources<'_>,
        drawn: &dyn Fn(DrawKind) -> bool,
        output: &mut u32,
    ) {
        let first_job = self.jobs.len() as u32;
        let mut max_vertices = 0;
        let glint_keys = [false, true].map(|item| {
            source
                .glint(item)
                .map(|texture| self.view(LayerTexture { texture, layer: 0 }))
        });
        source.for_each_instance(&mut |instance| {
            let data = instance.data;
            let Some(class) = classify(data.material, data.glint[5], instance.blend, hand) else {
                return;
            };
            let camera = (instance.camera && drawn(class.kind)).then_some(class.kind);
            let shadow = class.shadow && drawn(DrawKind::ShadowEntities);
            if (camera.is_none() && !shadow) || instance.vertex_count < 3 {
                return;
            }
            let vertex_count = instance.vertex_count - instance.vertex_count % 3;
            let texture = self.view(instance.texture);
            let first = *output;
            let job = SkinJob {
                instance: instance.index,
                first_output: first,
                vertex_count,
                flags: if hand { JOB_VIEW_SPACE } else { 0 }
                    | if class.one_sided { JOB_ONE_SIDED } else { 0 },
                light: u32::from(instance.light[0].min(15))
                    | (u32::from(instance.light[1].min(15)) << 4),
                colour: vertex_colour(
                    data.tint,
                    matches!(
                        class.kind,
                        DrawKind::EntitiesTranslucent | DrawKind::HandWater
                    ),
                ),
                block_id: -1,
                pad: 0,
            };
            *output += job.output_vertices();
            max_vertices = max_vertices.max(vertex_count);
            self.jobs.push(job);
            let entity_id = instance
                .runtime_id
                .zip(sources.entity_ids)
                .map_or(-1, |(runtime_id, ids)| ids.entity_id(runtime_id));
            let item_id = match instance.held {
                Some(Hand::Main) => sources.held_items[0],
                Some(Hand::Off) => sources.held_items[1],
                None => -1,
            };
            let values = DrawValues {
                entity_id,
                item_id,
                entity_color: entity_colour(data.overlay_rgba8, data.material),
                ..DrawValues::NONE
            };
            let vertices = first..*output;
            if let Some(kind) = camera {
                self.push(kind, vertices.clone(), texture, values);
                let item = data.glint[5] > 0.0 || instance.held.is_some();
                if class.glint
                    && drawn(DrawKind::ArmorGlint)
                    && let Some(glint) = glint_keys[usize::from(item)]
                {
                    let scale = if item {
                        ITEM_GLINT_SCALE
                    } else {
                        ENTITY_GLINT_SCALE
                    };
                    let texture_matrix = glint_texture_matrix(sources.seconds, scale);
                    self.push(
                        DrawKind::ArmorGlint,
                        vertices.clone(),
                        glint,
                        DrawValues {
                            texture_matrix,
                            ..values
                        },
                    );
                }
            }
            if shadow {
                self.push(DrawKind::ShadowEntities, vertices, texture, values);
            }
        });
        let count = self.jobs.len() as u32 - first_job;
        if count > 0 {
            self.dispatches.push(Dispatch::Rig {
                hand,
                first: first_job,
                count,
                max_vertices,
            });
        }
    }

    /// Plans the block-entity conversions and draws.
    fn plan_block_entities(
        &mut self,
        source: &BlockEntitySource<'_>,
        drawn: &dyn Fn(DrawKind) -> bool,
        output: &mut u32,
    ) {
        let texture = self.view(LayerTexture {
            texture: source.atlas,
            layer: 0,
        });
        let lists = [
            (source.solid, DrawKind::Block, true),
            (source.overlay, DrawKind::BlockTranslucent, false),
            (source.additive, DrawKind::BeaconBeam, false),
        ];
        for (list_index, (list, kind, casts)) in lists.into_iter().enumerate() {
            let Some(list) = list.filter(|list| list.count >= 3) else {
                continue;
            };
            let shadow = casts && drawn(DrawKind::ShadowBlock);
            if !drawn(kind) && !shadow {
                continue;
            }
            let count = list.count - list.count % 3;
            let first = *output;
            *output += count;
            self.dispatches.push(Dispatch::BlockEntities {
                list: list_index,
                first_output: first,
                count,
            });
            let vertices = first..*output;
            if drawn(kind) {
                self.push(kind, vertices.clone(), texture, DrawValues::NONE);
            }
            if shadow {
                self.push(DrawKind::ShadowBlock, vertices, texture, DrawValues::NONE);
            }
        }
    }
}

/// Plans, uploads and skins this frame's actor geometry for `state`'s view, and updates the
/// draws' uniforms and bind groups. Draw uniforms must have been written for this frame.
pub(crate) fn prepare(
    device: &Device,
    queue: &Queue,
    encoder: &mut CommandEncoder,
    state: &mut ViewState,
    sources: &ActorSources<'_>,
) {
    let pack = Arc::clone(&state.pack);
    let ViewState {
        actors,
        targets,
        bindings,
        ..
    } = state;
    actors.begin();
    let Some(kernels) = sources.kernels else {
        return;
    };
    let drawn = |kind: DrawKind| pack.draws.iter().any(|d| d.kind == kind);
    if !DrawKind::ALL
        .into_iter()
        .any(|kind| kind.actor() && drawn(kind))
    {
        return;
    }
    let mut output = 0u32;
    if let Some(source) = sources.actors {
        actors.plan_rig(source, false, sources, &drawn, &mut output);
    }
    if let Some(source) = sources.hand {
        actors.plan_rig(source, true, sources, &drawn, &mut output);
    }
    if let Some(source) = &sources.block_entities {
        actors.plan_block_entities(source, &drawn, &mut output);
    }
    let frame = actors.frame;
    actors.views.retain(|_, (_, used)| *used == frame);
    if output == 0 {
        actors.draws.clear();
        return;
    }
    skin(device, queue, encoder, actors, sources, kernels, output);
    let neutral = actors
        .neutral
        .get_or_insert_with(|| NeutralTextures::new(device, queue));
    let mut next_slot: HashMap<DrawKind, u32> = HashMap::new();
    for draw in &actors.draws {
        let Some(index) = pack.draws.iter().position(|d| d.kind == draw.kind) else {
            actors.slot_of.push(u32::MAX);
            continue;
        };
        let geometry = &pack.draws[index];
        let program = &pack.programs[geometry.program];
        let slot_index = next_slot.entry(draw.kind).or_default();
        let slots = actors.slots.entry(draw.kind).or_default();
        if slots.len() <= *slot_index as usize {
            let size = program
                .reflection
                .uniforms
                .as_ref()
                .map_or(16, |block| u64::from(block.size));
            slots.push(Slot {
                uniforms: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("shader pack {} actor uniforms", program.name)),
                    size: size.max(16).next_multiple_of(16),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                texture: None,
                groups: Default::default(),
            });
        }
        let slot = &mut slots[*slot_index as usize];
        actors.slot_of.push(*slot_index);
        *slot_index += 1;
        if let Some(block) = &program.reflection.uniforms {
            actors.scratch.clone_from(&bindings.draws[index].base);
            actors.scratch.resize(block.size as usize, 0);
            patch_block(block, &draw.values.uniforms(), &mut actors.scratch);
            queue.write_buffer(&slot.uniforms, 0, &actors.scratch);
        }
        if slot.texture != Some(draw.texture) {
            let Some((view, _)) = actors.views.get(&draw.texture) else {
                continue;
            };
            slot.groups = pass_bind_groups(
                device,
                &pack,
                targets,
                Some(&slot.uniforms),
                geometry.program,
                draw_read_alt(&pack, geometry),
                Some(AtlasViews {
                    color: view,
                    normals: &neutral.normals,
                    specular: &neutral.specular,
                }),
            );
            slot.texture = Some(draw.texture);
        }
    }
}

/// Uploads the jobs and records the skinning and block-entity dispatches into the output.
fn skin(
    device: &Device,
    queue: &Queue,
    encoder: &mut CommandEncoder,
    actors: &mut ViewActors,
    sources: &ActorSources<'_>,
    kernels: skin::SkinKernels<'_>,
    output: u32,
) {
    let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
    let output_buffer = actors
        .output
        .ensure(
            device,
            "shader-pack actor vertices",
            u64::from(output) * IRIS_VERTEX_STRIDE,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
        )
        .clone();
    let jobs = actors
        .job_buffer
        .ensure(
            device,
            "shader-pack actor skinning jobs",
            (actors.jobs.len().max(1) * std::mem::size_of::<SkinJob>()) as u64,
            storage,
        )
        .clone();
    queue.write_buffer(&jobs, 0, bytemuck::cast_slice(&actors.jobs));
    let stride =
        u64::from(device.limits().min_uniform_buffer_offset_alignment).max(SKIN_PARAMS_BYTES);
    let params = actors
        .params
        .ensure(
            device,
            "shader-pack actor skinning parameters",
            stride * actors.dispatches.len() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        )
        .clone();
    let camera = sources.camera.extend(0.0).to_array();
    let block_lists = sources
        .block_entities
        .map(|source| [source.solid, source.overlay, source.additive]);
    let mut groups = Vec::with_capacity(actors.dispatches.len());
    for (index, dispatch) in actors.dispatches.iter().enumerate() {
        let (counts, source_group) = match *dispatch {
            Dispatch::Rig {
                hand, first, count, ..
            } => {
                let source = if hand { sources.hand } else { sources.actors };
                let Some(source) = source else {
                    groups.push(None);
                    continue;
                };
                let buffers = source.buffers();
                let entries: Vec<_> = [
                    buffers.instances,
                    buffers.vertices,
                    buffers.spans,
                    buffers.previous_bones,
                    buffers.current_bones,
                ]
                .iter()
                .enumerate()
                .map(|(binding, buffer)| wgpu::BindGroupEntry {
                    binding: binding as u32,
                    resource: buffer.as_entire_binding(),
                })
                .collect();
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("shader-pack actor skinning sources"),
                    layout: kernels.rig_layout,
                    entries: &entries,
                });
                ([count, first, 0, 0], group)
            }
            Dispatch::BlockEntities {
                list,
                first_output,
                count,
            } => {
                let Some(list) = block_lists.and_then(|lists| lists[list]) else {
                    groups.push(None);
                    continue;
                };
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("shader-pack block-entity sources"),
                    layout: kernels.block_entity_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 5,
                        resource: list.buffer.as_entire_binding(),
                    }],
                });
                ([count, first_output, (-1i32).cast_unsigned(), 0], group)
            }
        };
        let offset = stride * index as u64;
        queue.write_buffer(
            &params,
            offset,
            bytemuck::bytes_of(&SkinParams { camera, counts }),
        );
        let output_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shader-pack actor skinning output"),
            layout: kernels.output_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: jobs.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &params,
                        offset,
                        size: std::num::NonZeroU64::new(SKIN_PARAMS_BYTES),
                    }),
                },
            ],
        });
        groups.push(Some((source_group, output_group)));
    }
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("shader-pack actor skinning"),
        timestamp_writes: None,
    });
    for (dispatch, groups) in actors.dispatches.iter().zip(&groups) {
        let Some((source_group, output_group)) = groups else {
            continue;
        };
        let (pipeline, [x, y, z]) = match *dispatch {
            Dispatch::Rig {
                count,
                max_vertices,
                ..
            } => (kernels.rig, rig_workgroups(count, max_vertices)),
            Dispatch::BlockEntities { count, .. } => {
                (kernels.block_entities, linear_workgroups(count))
            }
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, source_group, &[]);
        pass.set_bind_group(1, output_group, &[]);
        pass.dispatch_workgroups(x, y, z);
    }
}

/// Workgroups of a rig dispatch: rig vertices in x, jobs in y, continuing in z.
pub(crate) fn rig_workgroups(jobs: u32, max_vertices: u32) -> [u32; 3] {
    let x = max_vertices.div_ceil(SKIN_WORKGROUP_SIZE).max(1);
    let y = jobs.clamp(1, MAX_WORKGROUPS);
    [x, y, jobs.div_ceil(y).max(1)]
}

/// Workgroups covering `count` invocations, continuing in y past the x limit.
pub(crate) fn linear_workgroups(count: u32) -> [u32; 3] {
    let groups = count.div_ceil(SKIN_WORKGROUP_SIZE).max(1);
    let x = groups.min(MAX_WORKGROUPS);
    [x, groups.div_ceil(x), 1]
}

/// Records the pack's draw of `kind` over every actor draw of that kind.
pub(crate) fn record(
    encoder: &mut CommandEncoder,
    state: &mut ViewState,
    kind: DrawKind,
    write_alt: u32,
    depth: Option<(&TextureView, [u32; 2])>,
) {
    if !state.actors.draws.iter().any(|draw| draw.kind == kind) {
        return;
    }
    let Some(output) = state.actors.output.buffer.clone() else {
        return;
    };
    let constant = state.pack.host.constant_stream.clone();
    let Some((mut render, _)) = open_kind(encoder, state, kind, write_alt, depth) else {
        return;
    };
    render.set_vertex_buffer(GEOMETRY_VERTEX_BUFFER, output.slice(..));
    render.set_vertex_buffer(CONSTANT_VERTEX_BUFFER, constant.slice(..));
    let actors = &state.actors;
    let Some(slots) = actors.slots.get(&kind) else {
        return;
    };
    for (draw, slot) in actors.draws.iter().zip(&actors.slot_of) {
        if draw.kind != kind {
            continue;
        }
        let Some(slot) = slots
            .get(*slot as usize)
            .filter(|slot| slot.texture == Some(draw.texture))
        else {
            continue;
        };
        for (index, group) in slot.groups.iter().enumerate() {
            if let Some(group) = group {
                render.set_bind_group(index as u32, group, &[]);
            }
        }
        render.draw(draw.vertices.clone(), 0..1);
    }
}
