//! Opt-in camera-local hand rigs reuse the actor owner's geometry and pose admission.

use crate::actors::{ActorCoverage, ActorInstance, ActorSample, geometry, pose};
use recording_studio::{
    archive::{ArchiveError, MAX_RECORD_BYTES},
    object_store::{AssetId, AssetReader, AssetWriter},
};
use render_api::SkinRgba8;
use render_model::hand::{HAND_ITEM_LAYER_FLAG, HAND_OFFHAND_LAYER_FLAG, HAND_TEXTURE_LAYER_MASK};
use render_model::{
    ActorRigGeometrySpan, ActorRigRenderFrame, ActorRigVertexSegments, EquipmentRaster,
    HandItemAtlas, HandRigLight,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub struct PreparedHand {
    pub rig: ActorRigRenderFrame,
    pub skin: SkinRgba8,
    pub light: HandRigLight,
    pub fov_radians: f32,
    pub revision: u64,
    pub item_atlases: [Option<HandItemAtlas>; 2],
    pub item_glint: Option<EquipmentRaster>,
}

impl PreparedHand {
    /// Includes static catalog pages because the queued hand may retain a replaced geometry epoch.
    pub(super) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.skin.len()
            + self
                .rig
                .geometry_vertices
                .segments
                .iter()
                .map(|p| std::mem::size_of_val(&**p))
                .sum::<usize>()
            + std::mem::size_of_val(&*self.rig.geometry_vertices.segments)
            + std::mem::size_of_val(&*self.rig.geometry_vertices.offsets)
            + std::mem::size_of_val(&*self.rig.geometry_spans)
            + std::mem::size_of_val(&*self.rig.instances)
            + std::mem::size_of_val(&*self.rig.manifest)
            + std::mem::size_of_val(&*self.rig.previous_bones)
            + std::mem::size_of_val(&*self.rig.current_bones)
            + self
                .item_atlases
                .iter()
                .flatten()
                .map(|a| a.rgba8.len())
                .sum::<usize>()
            + self.item_glint.as_ref().map_or(0, |r| r.rgba8.len())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Metadata {
    skin: AssetId,
    skin_bytes: usize,
    light: [u32; std::mem::size_of::<HandRigLight>() / 4],
    fov_radians: f32,
    revision: u64,
    atlases: [Option<Raster>; 2],
    glint: Option<Raster>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raster {
    width: u16,
    height: u16,
    layers: u32,
    pixels: AssetId,
}

#[derive(Default)]
pub(super) struct GeometryCache(
    Option<(ActorRigVertexSegments, Arc<[ActorRigGeometrySpan]>, AssetId)>,
);

/// Retains at most one catalog epoch; unchanged immutable geometry is serialized only once.
pub(super) fn store(
    store: &mut AssetWriter,
    cache: &mut GeometryCache,
    hand: &PreparedHand,
) -> Result<(Metadata, Vec<u8>), ArchiveError> {
    let rig = &hand.rig;
    let geometry = match &cache.0 {
        Some((vertices, spans, id))
            if vertices.epoch == rig.geometry_vertices.epoch
                && Arc::ptr_eq(&vertices.segments, &rig.geometry_vertices.segments)
                && Arc::ptr_eq(&vertices.offsets, &rig.geometry_vertices.offsets)
                && Arc::ptr_eq(spans, &rig.geometry_spans) =>
        {
            *id
        }
        _ => {
            let id = geometry::store(store, &rig.geometry_vertices, &rig.geometry_spans)?;
            cache.0 = Some((
                rig.geometry_vertices.clone(),
                Arc::clone(&rig.geometry_spans),
                id,
            ));
            id
        }
    };
    let mut atlases: [Option<Raster>; 2] = [None, None];
    for (out, atlas) in atlases.iter_mut().zip(&hand.item_atlases) {
        *out = atlas
            .as_ref()
            .map(|a| {
                Ok::<_, ArchiveError>(Raster {
                    width: a.width,
                    height: a.height,
                    layers: a.layers,
                    pixels: store.put_shared_pixels(&a.rgba8)?,
                })
            })
            .transpose()?;
    }
    let metadata = Metadata {
        skin: store.put_shared_pixels(hand.skin.pixels())?,
        skin_bytes: hand.skin.len(),
        light: bytemuck::cast(hand.light),
        fov_radians: hand.fov_radians,
        revision: hand.revision,
        atlases,
        glint: hand
            .item_glint
            .as_ref()
            .map(|r| {
                Ok::<_, ArchiveError>(Raster {
                    width: r.width,
                    height: r.height,
                    layers: 1,
                    pixels: store.put_shared_pixels(&r.rgba8)?,
                })
            })
            .transpose()?,
    };
    let instances = rig
        .instances
        .iter()
        .zip(rig.manifest.iter())
        .map(|(instance, entry)| ActorInstance {
            runtime_id: entry.identity.runtime_id,
            position: instance.world_from_actor.map(|r| r[3]),
            pitch_radians: 0.0,
            yaw_radians: 0.0,
            head_yaw_radians: 0.0,
            skin_layer: instance.texture_layer,
        })
        .collect::<Vec<_>>()
        .into();
    let sample = ActorSample {
        coverage: ActorCoverage::OriginalView,
        instances,
        instance_revision: hand.revision,
        skin_revision: hand.revision,
        rig: rig.clone(),
        instance_pages: vec![0; rig.instances.len()].into(),
    };
    let mut bytes = Vec::new();
    pose::write(&mut bytes, geometry, AssetId([0; 32]), &sample)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(ArchiveError::Quota);
    }
    Ok((metadata, bytes))
}

/// Geometry caches are limited to one selected hand epoch, independently of the live player.
pub(super) struct Loader {
    geometry: Option<(AssetId, geometry::LoadedGeometry)>,
}
impl Default for Loader {
    fn default() -> Self {
        Self { geometry: None }
    }
}

impl Loader {
    /// Validates all arm and item selector bits against independently hash-checked skin and atlas rasters.
    pub(super) fn load(
        &mut self,
        store: &AssetReader,
        metadata: Metadata,
        bytes: &[u8],
        cancelled: &impl Fn() -> bool,
        mut pixels: impl FnMut(AssetId, usize) -> Result<Arc<[u8]>, ArchiveError>,
    ) -> Result<PreparedHand, ArchiveError> {
        let m = metadata;
        let skin: SkinRgba8 = pixels(m.skin, m.skin_bytes)?.into();
        let light: HandRigLight = bytemuck::cast(m.light);
        if render_model::actor_skin_side(&skin).is_none()
            || !m.fov_radians.is_finite()
            || !(0.0..std::f32::consts::PI).contains(&m.fov_radians)
            || m.revision == 0
            || light.block_level > 15
            || light.sky_level > 15
            || !light.daylight.is_finite()
            || !light
                .java_lights
                .iter()
                .chain(&light.java_normal_axes)
                .flatten()
                .all(|v| v.is_finite())
        {
            return Err(ArchiveError::Invalid(
                "invalid recorded hand skin, projection or lighting",
            ));
        }
        let mut atlases: [Option<HandItemAtlas>; 2] = [None, None];
        for (out, raster) in atlases.iter_mut().zip(m.atlases) {
            *out = raster
                .map(|r| {
                    Ok::<_, ArchiveError>(HandItemAtlas {
                        width: r.width,
                        height: r.height,
                        layers: r.layers,
                        rgba8: pixels(r.pixels, raster_bytes(&r)?)?,
                    })
                })
                .transpose()?;
        }
        let glint = m
            .glint
            .map(|r| {
                if r.layers != 1 {
                    return Err(ArchiveError::Invalid("invalid recorded hand glint layers"));
                }
                Ok(EquipmentRaster {
                    width: r.width,
                    height: r.height,
                    rgba8: pixels(r.pixels, raster_bytes(&r)?)?,
                })
            })
            .transpose()?;
        let pending = pose::read(&mut &bytes[..])?;
        if self
            .geometry
            .as_ref()
            .is_none_or(|(id, _)| *id != pending.geometry)
        {
            self.geometry = Some((
                pending.geometry,
                geometry::load(store, pending.geometry, cancelled)?,
            ));
        }
        let sample = pending.admit_rig(
            &self.geometry.as_ref().expect("admitted hand geometry").1,
            |instance, page| {
                let selector = instance.texture_layer;
                if page != 0 {
                    return Err(ArchiveError::Invalid(
                        "recorded hand has an actor artwork page",
                    ));
                }
                if selector & HAND_ITEM_LAYER_FLAG != 0 {
                    let offhand = usize::from(selector & HAND_OFFHAND_LAYER_FLAG != 0);
                    if atlases[offhand]
                        .as_ref()
                        .is_none_or(|a| (selector & HAND_TEXTURE_LAYER_MASK) >= a.layers)
                    {
                        return Err(ArchiveError::Invalid(
                            "recorded hand item texture is missing",
                        ));
                    }
                } else if selector != 0 {
                    return Err(ArchiveError::Invalid("invalid recorded hand skin selector"));
                }
                Ok(())
            },
        )?;
        if cancelled() {
            return Err(ArchiveError::Cancelled);
        }
        if sample.rig.instances.is_empty() {
            return Err(ArchiveError::Invalid("empty recorded hand"));
        }
        Ok(PreparedHand {
            rig: sample.rig,
            skin,
            light,
            fov_radians: m.fov_radians,
            revision: m.revision,
            item_atlases: atlases,
            item_glint: glint,
        })
    }
}

/// Atlas products use checked dimensions before pixel allocation and the frame's cumulative budget.
fn raster_bytes(r: &Raster) -> Result<usize, ArchiveError> {
    usize::from(r.width)
        .checked_mul(usize::from(r.height))
        .and_then(|v| v.checked_mul(r.layers as usize))
        .and_then(|v| v.checked_mul(4))
        .filter(|&v| v > 0 && v <= super::MAX_PIXELS)
        .ok_or(ArchiveError::Quota)
}
