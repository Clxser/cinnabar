//! Publishes dropped items, falling blocks, primed TNT and ropes as renderer geometry.
use std::sync::Arc;

use assets::{
    BlockFace, ItemVisualRoute, MATERIAL_FLAG_FOLIAGE_TINT, MATERIAL_FLAG_GRASS_TINT,
    MATERIAL_FLAG_TINT_MASK, MATERIAL_FLAG_WATER_TINT, NetworkIdMode, RuntimeAssets, VisualKind,
};
use bevy::{
    ecs::system::SystemParam,
    platform::collections::HashMap,
    prelude::{Camera, Camera3d, Local, Query, Res, ResMut, With},
};
use chunk_pipeline::WorldStream;
use client_world::{BlockEntityKind, RopeKind};
use render_model::{DroppedItemBlock, DroppedItemCube, DroppedItemSprite};
use {
    render_model::{
        ActorCullView, DroppedItemInstance, DroppedItemModel, DroppedItemShape, ItemMeshVertex,
        MAX_ITEM_SPRITE_SIDE, TerrainItemInstance, TerrainItemTransition, dropped_item_transform,
        pack_overlay_rgba8, rope_color, rope_ribbon,
    },
    render_scene::{ChunkTextureAssets, DroppedItemScene},
};

use client_ui::ui_runtime::presentation::UiPresentationRuntime;

mod dials;
mod frame;
mod lighting;
mod preparation;
pub mod recording;
use preparation::DropPreparation;

// Provisional world sizes and colours; each needs independent measurement.
const TNT_FLASH_OVERLAY: [f32; 4] = [1.0, 1.0, 1.0, 0.8];
const FISHING_SEGMENTS: usize = 16;
const FISHING_SAG_FRACTION: f32 = 0.1;
const FISHING_HALF_WIDTH: f32 = 0.006;
const LEAD_SEGMENTS: usize = 16;
const LEAD_SAG_FRACTION: f32 = 0.12;
const LEAD_HALF_WIDTH: f32 = 0.0125;
const GRASS_TINT_RGB: [u8; 3] = [0x79, 0xc0, 0x5a];
const FOLIAGE_TINT_RGB: [u8; 3] = [0x77, 0xab, 0x2f];
const WATER_TINT_RGB: [u8; 3] = [0x3f, 0x76, 0xe4];
/// Daylight scale until the celestial curve feeds world-space actors.
pub(super) const DAYLIGHT: f32 = 1.0;

mod cache;
use cache::{ModelCache, ModelKey};

#[derive(SystemParam)]
pub(super) struct DroppedItemPublisher<'w, 's> {
    scene: Option<ResMut<'w, DroppedItemScene>>,
    textures: Option<Res<'w, ChunkTextureAssets>>,
    placements: Option<Res<'w, render_scene::StaticItemPlacementsResource>>,
    cache: Local<'s, ModelCache>,
    replay: Option<ResMut<'w, recording::ReplayItems>>,
    scratch: Local<'s, PublishScratch>,
    device: Option<Res<'w, bevy::render::renderer::RenderDevice>>,
    cameras: Query<
        'w,
        's,
        (
            &'static Camera,
            Option<&'static bevy::render::camera::TemporalJitter>,
        ),
        With<Camera3d>,
    >,
}

/// Frame-local buffers kept between publications; every field is cleared before it is filled,
/// so only retained capacity carries over.
#[derive(Default)]
pub(crate) struct PublishScratch {
    instances: Vec<DroppedItemInstance>,
    terrain_instances: Vec<TerrainItemInstance>,
    lines: Vec<ItemMeshVertex>,
    /// The icon each static placement (by index) resolved during demand collection.
    placement_icons: Vec<Option<ui::IconRef>>,
    transitions: HashMap<i64, Vec<TerrainItemTransition>>,
    /// This frame's dropped-item views, block actors and ropes from the world authority;
    /// emptied again once published so no view outlives its frame.
    dropped: Vec<client_world::DroppedItemView>,
    candidates: Vec<client_world::BlockEntityCandidate>,
    ropes: Vec<client_world::RopeView>,
}

fn tint_rgba(flags: u32) -> u32 {
    let [r, g, b] = match flags & MATERIAL_FLAG_TINT_MASK {
        MATERIAL_FLAG_GRASS_TINT => GRASS_TINT_RGB,
        MATERIAL_FLAG_FOLIAGE_TINT => FOLIAGE_TINT_RGB,
        MATERIAL_FLAG_WATER_TINT => WATER_TINT_RGB,
        _ => [255; 3],
    };
    rope_color(r, g, b)
}

fn item_block_id(stream: &WorldStream, visual: ItemVisualRoute) -> Option<(NetworkIdMode, u32)> {
    match visual {
        ItemVisualRoute::BlockItem(id) => Some((NetworkIdMode::Sequential, id.0)),
        ItemVisualRoute::RetainedBlock { block_runtime_id } => Some((
            stream.network_id_mode(),
            stream.resolve_block_network_id(u32::from_ne_bytes(block_runtime_id.to_ne_bytes())),
        )),
        _ => None,
    }
}

fn entity_block_id(
    stream: &WorldStream,
    kind: &BlockEntityKind,
) -> Option<(NetworkIdMode, u32, f32)> {
    match kind {
        BlockEntityKind::Falling { block_runtime_id } => Some((
            stream.network_id_mode(),
            stream.resolve_block_network_id(u32::from_ne_bytes(block_runtime_id.to_ne_bytes())),
            1.0,
        )),
        BlockEntityKind::PrimedTnt { visual } => match visual {
            ItemVisualRoute::BlockItem(id) => Some((NetworkIdMode::Sequential, id.0, 1.0)),
            _ => None,
        },
    }
}

/// Builds a unit cube from a cube-kind block's six face textures, or `None` for other kinds.
fn block_cube(assets: &RuntimeAssets, mode: NetworkIdMode, id: u32) -> Option<DroppedItemCube> {
    let block = assets.resolve(mode, id);
    if !block.is_known() || block.kind() != VisualKind::Cube {
        return None;
    }
    let mut tile_size = None;
    let mut faces: Vec<Arc<[u8]>> = Vec::with_capacity(6);
    let mut tints = [0_u32; 6];
    for (index, face) in BlockFace::ALL.into_iter().enumerate() {
        let material = assets.material(block.face(face).material_id());
        let page = assets
            .texture_pages()
            .get(material.texture.page() as usize)?;
        let mip = page.texture.mips.first()?;
        let size = mip.size;
        if size == 0 || size > MAX_ITEM_SPRITE_SIDE || *tile_size.get_or_insert(size) != size {
            return None;
        }
        let bytes = (size * size * 4) as usize;
        if bytes.checked_mul(BlockFace::ALL.len())? > assets::texture::policy::MAX_SUBSCRIBER_BYTES
        {
            return None;
        }
        let start = material.texture.layer() as usize * bytes;
        faces.push(Arc::from(mip.rgba8.get(start..start + bytes)?));
        tints[index] = tint_rgba(material.flags);
    }
    Some(DroppedItemCube {
        tile: tile_size?,
        faces: faces.try_into().ok()?,
        tints,
    })
}

/// Counts the distinct texture materials needed by a block model without copying its data.
fn block_model_layers(assets: &RuntimeAssets, mode: NetworkIdMode, id: u32) -> Option<usize> {
    let block = assets.resolve(mode, id);
    if !block.is_known() {
        return None;
    }
    if block.kind() == VisualKind::Cube {
        return Some(BlockFace::ALL.len());
    }
    if !matches!(block.kind(), VisualKind::Model | VisualKind::Cross) {
        return None;
    }
    template_layers(assets, block.model_template()?)
}

/// Counts materials in one model's contiguous parts without copying texture pixels.
fn template_layers(assets: &RuntimeAssets, mut template_id: u32) -> Option<usize> {
    let mut materials = std::collections::HashSet::new();
    loop {
        let template = assets.model_templates().get(template_id as usize)?;
        let first = template.quad_start as usize;
        for quad in assets
            .model_quads()
            .get(first..first + template.quad_count as usize)?
        {
            materials.insert(quad.material);
        }
        if template.flags & assets::MODEL_TEMPLATE_FLAG_COMPOUND_NEXT == 0 {
            return Some(materials.len());
        }
        template_id = template_id.checked_add(1)?;
    }
}

fn block_template(
    assets: &RuntimeAssets,
    mode: NetworkIdMode,
    id: u32,
) -> Option<DroppedItemBlock> {
    let block = assets.resolve(mode, id);
    if !block.is_known() || !matches!(block.kind(), VisualKind::Model | VisualKind::Cross) {
        return None;
    }
    template_model(assets, block.model_template()?, block.variant() & 3)
}

/// Retains the authored quads and pixels for a selected world or pot-content template.
fn template_model(
    assets: &RuntimeAssets,
    mut template_id: u32,
    rotation: u32,
) -> Option<DroppedItemBlock> {
    let mut quads = Vec::new();
    let mut materials = Vec::new();
    let mut material_indices = HashMap::new();
    let mut pixel_bytes = 0;
    loop {
        let template = assets.model_templates().get(template_id as usize)?;
        let first = template.quad_start as usize;
        for quad in assets
            .model_quads()
            .get(first..first + template.quad_count as usize)?
        {
            let mut quad = *quad;
            let material_index = if let Some(index) = material_indices.get(&quad.material) {
                *index
            } else {
                let material = assets.material(quad.material);
                let page = assets
                    .texture_pages()
                    .get(material.texture.page() as usize)?;
                let mip = page.texture.mips.first()?;
                let size = mip.size;
                if size == 0 || size > assets::MAX_TILE_SIZE {
                    return None;
                }
                let bytes = (size * size * 4) as usize;
                pixel_bytes += bytes;
                if pixel_bytes > assets::texture::policy::MAX_SUBSCRIBER_BYTES {
                    return None;
                }
                let first = material.texture.layer() as usize * bytes;
                let index = materials.len() as u32;
                materials.push((
                    DroppedItemSprite {
                        width: size,
                        height: size,
                        rgba8: Arc::from(mip.rgba8.get(first..first + bytes)?),
                    },
                    tint_rgba(material.flags),
                ));
                material_indices.insert(quad.material, index);
                index
            };
            let grid = assets.texture_source_grid(assets.material(quad.material).texture);
            for uv in &mut quad.uvs {
                *uv = uv.map(|axis| axis >> grid);
            }
            quad.material = material_index;
            quads.push(quad);
        }
        if template.flags & assets::MODEL_TEMPLATE_FLAG_COMPOUND_NEXT == 0 {
            break;
        }
        template_id = template_id.checked_add(1)?;
    }
    Some(DroppedItemBlock {
        materials: materials.into(),
        quads: quads.into(),
        rotation,
    })
}

impl DroppedItemPublisher<'_, '_> {
    /// Retains draws for multiple cameras, jitter or subviews absent from the publication view.
    fn culling_view(
        &self,
        view: Option<ActorCullView>,
    ) -> Option<render_model::dropped_item::DroppedItemFrustum> {
        preparation::publication_frustum(
            view,
            self.cameras
                .single()
                .is_ok_and(|(camera, jitter)| jitter.is_none() && camera.sub_camera_view.is_none()),
        )
    }

    /// Resolves a plant's separate potted geometry through the current terrain carrier.
    fn potted_model(cache: &mut ModelCache, assets: &RuntimeAssets, id: u32) -> Option<u32> {
        let template = assets.potted_model_template(NetworkIdMode::Sequential, id)?;
        cache.model(
            ModelKey::PottedBlock { id },
            || template_layers(assets, template),
            || template_model(assets, template, 0).map(DroppedItemModel::Block),
        )
    }
    fn block_model(
        cache: &mut ModelCache,
        assets: &RuntimeAssets,
        mode: NetworkIdMode,
        id: u32,
    ) -> Option<u32> {
        let key = ModelKey::Block {
            hashed: mode == NetworkIdMode::Hashed,
            id,
        };
        cache.model(
            key,
            || block_model_layers(assets, mode, id),
            || {
                block_cube(assets, mode, id)
                    .map(DroppedItemModel::Cube)
                    .or_else(|| block_template(assets, mode, id).map(DroppedItemModel::Block))
            },
        )
    }

    /// Resolves a sprite model, reusing its image and placement identity.
    fn icon_model(
        cache: &mut ModelCache,
        icons: &UiPresentationRuntime,
        identifier: &str,
        metadata: u32,
        native_drop: bool,
    ) -> Option<u32> {
        let icon = icons.item_icon(identifier, metadata)?;
        Self::resolved_icon_model(cache, icon, native_drop, || {
            icon_sprite(icons, identifier, metadata, native_drop)
        })
    }

    /// Resolves the sprite model of an already looked-up `icon`; `build` produces its pixels
    /// from the same identifier and metadata only when the model is not resident.
    fn resolved_icon_model(
        cache: &mut ModelCache,
        icon: ui::IconRef,
        native_drop: bool,
        build: impl FnOnce() -> Option<DroppedItemModel>,
    ) -> Option<u32> {
        cache.model(ModelKey::icon(icon, native_drop), || Some(1), build)
    }

    fn carried_block_model(
        cache: &mut ModelCache,
        icons: &UiPresentationRuntime,
        assets: &RuntimeAssets,
        mode: NetworkIdMode,
        id: u32,
    ) -> Option<u32> {
        let key = ModelKey::CarriedBlock {
            hashed: mode == NetworkIdMode::Hashed,
            id,
        };
        cache.model(
            key,
            || Some(BlockFace::ALL.len()),
            || {
                let block = assets.resolve(mode, id);
                if !block.is_known() {
                    return None;
                }
                let visual = match mode {
                    NetworkIdMode::Sequential => id,
                    NetworkIdMode::Hashed => assets.sequential_id_for_hash(id)?,
                };
                if block.kind() != VisualKind::Cube
                    && assets::gui_item::full_cube_model_quads(
                        assets,
                        assets::BlockVisualId(visual),
                    )
                    .is_none()
                {
                    return None;
                }
                let cube =
                    icons.carried_block_cube(visual, assets.provenance().source_manifest_sha256)?;
                Some(DroppedItemModel::Cube(cube))
            },
        )
    }

    pub(super) fn publish(
        &mut self,
        icons: Option<&UiPresentationRuntime>,
        stream: Option<&WorldStream>,
        collisions: Option<&dyn crate::observations::CollisionLookup>,
        camera: (Option<([f32; 3], f32)>, Option<ActorCullView>),
        partial_tick: f32,
        glint: (
            Option<render_model::EquipmentRaster>,
            render_model::ActorGlint,
        ),
    ) {
        let (camera, cull_view) = camera;
        let frustum = self.culling_view(cull_view);
        let (Some(scene), Some(icons)) = (self.scene.as_mut(), icons) else {
            return;
        };
        let Some(stream) = stream else {
            scene.clear();
            return;
        };
        Self::prepare_scene(
            scene,
            &mut self.cache,
            self.textures.as_deref(),
            self.placements
                .as_ref()
                .map_or(&[], |placements| &placements.0.0),
            self.device.as_deref(),
            icons,
            stream,
            collisions,
            camera,
            frustum,
            partial_tick,
            glint,
            false,
        );
        if let Some(replay) = self.replay.as_deref_mut() {
            replay.remember_source_poses(&self.cache.spawn_poses, stream, partial_tick);
        }
    }
}

/// Builds icon pixels lazily; unavailable sprites are retried on the next frame.
fn icon_sprite(
    icons: &UiPresentationRuntime,
    identifier: &str,
    metadata: u32,
    native_drop: bool,
) -> Option<DroppedItemModel> {
    let pixels = icons.item_sprite(
        identifier,
        metadata,
        assets::texture::policy::device_policy()
            .max_side()
            .min(MAX_ITEM_SPRITE_SIDE),
    )?;
    let sprite = DroppedItemSprite {
        width: pixels.width,
        height: pixels.height,
        rgba8: Arc::from(pixels.rgba8),
    };
    Some(if native_drop {
        DroppedItemModel::NativeSprite(sprite)
    } else {
        DroppedItemModel::Sprite(sprite)
    })
}

/// The icon identity dropped `item` (with `identifier`) draws: a dial's current strip frame,
/// or its ordinary icon.
fn dropped_icon_key<'a>(
    icons: &UiPresentationRuntime,
    identifier: &'a str,
    item: &client_world::CanonicalItemStack,
) -> (&'a str, u32) {
    icons
        .dial_icon_key(identifier, item.identity.metadata)
        .unwrap_or_else(|| {
            UiPresentationRuntime::item_icon_key(
                identifier,
                item.identity.metadata,
                item.use_animation,
                item.charged_projectile_role,
                None,
            )
        })
}

#[cfg(test)]
#[path = "dropped_items/tests.rs"]
mod palette_tests;
