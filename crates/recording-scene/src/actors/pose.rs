//! Bounded dynamic actor records; static geometry and raster objects are referenced by digest.

use std::{
    collections::BTreeSet,
    io::{BufReader, Read, Write},
    sync::Arc,
};

use flate2::{Compression, bufread::ZlibDecoder, write::ZlibEncoder};
use recording_studio::{archive::ArchiveError, object_store::AssetId};
use render_model::{
    ACTOR_GPU_INSTANCE_WORDS, ActorGpuInstance, ActorRigRenderFrame, ActorRigRoute,
    MAX_ACTOR_BONE_ARENA_BYTES, MAX_ACTOR_RENDER_INSTANCES, MAX_ACTOR_RIG_VERTICES,
    MAX_RENDER_BONES_PER_ACTOR,
};

use super::{
    ActorSample, FrozenActors, appearance::LoadedAppearance, geometry::LoadedGeometry, io_error,
    metadata::Metadata,
};
use crate::binary::{self, Reader};

const MAGIC: [u8; 8] = *b"CINAPS01";
const MAX_METADATA: usize = 2 * 1024 * 1024;
const MAX_RAW_BYTES: usize = MAX_ACTOR_BONE_ARENA_BYTES + MAX_METADATA + 1024 * 1024;
type Bone = [[f32; 4]; 3];
const BONE_WORDS: usize = std::mem::size_of::<Bone>() / std::mem::size_of::<u32>();

pub(crate) struct PendingPose {
    pub geometry: AssetId,
    pub appearance: AssetId,
    metadata: Metadata,
    instances: Vec<ActorGpuInstance>,
    previous_bones: Vec<Bone>,
    current_bones: Vec<Bone>,
    pages: Vec<u16>,
}

/// Compression stays on the journal worker. Dynamic poses never consume one object file per frame.
pub(crate) fn write(
    output: &mut dyn Write,
    geometry: AssetId,
    appearance: AssetId,
    sample: &ActorSample,
) -> Result<(), ArchiveError> {
    if sample.rig.instances.len() > MAX_ACTOR_RENDER_INSTANCES
        || sample.instances.len() != sample.rig.instances.len()
        || sample.rig.manifest.len() != sample.rig.instances.len()
        || sample.instance_pages.len() != sample.rig.instances.len()
        || (std::mem::size_of_val(&*sample.rig.previous_bones)
            + std::mem::size_of_val(&*sample.rig.current_bones))
            > MAX_ACTOR_BONE_ARENA_BYTES
    {
        return Err(ArchiveError::Invalid(
            "prepared actor sample exceeds native frame bounds",
        ));
    }
    let metadata = serde_json::to_vec(&Metadata::capture(sample))?;
    if metadata.len() > MAX_METADATA {
        return Err(ArchiveError::Quota);
    }
    let mut output = ZlibEncoder::new(output, Compression::fast());
    output
        .write_all(&MAGIC)
        .and_then(|()| output.write_all(&geometry.0))
        .and_then(|()| output.write_all(&appearance.0))
        .map_err(io_error)?;
    binary::blob(&mut output, &metadata)?;
    binary::packed(&mut output, &sample.rig.instances, |instance| {
        bytemuck::cast::<ActorGpuInstance, [u32; ACTOR_GPU_INSTANCE_WORDS]>(*instance)
    })?;
    for bones in [&sample.rig.previous_bones, &sample.rig.current_bones] {
        binary::packed(&mut output, bones, |bone| {
            bytemuck::cast::<Bone, [u32; BONE_WORDS]>(*bone)
        })?;
    }
    binary::packed(&mut output, &sample.instance_pages, |page| {
        [u32::from(*page)]
    })?;
    output.finish().map_err(io_error)?;
    Ok(())
}

/// Both compressed and decompressed envelopes reject trailing bytes, counts and incomplete streams.
pub(crate) fn read(input: &mut dyn Read) -> Result<PendingPose, ArchiveError> {
    let mut decoded = ZlibDecoder::new(BufReader::new(input));
    let mut reader = Reader::with_limit(&mut decoded, MAX_RAW_BYTES);
    if reader.bytes::<8>()? != MAGIC {
        return Err(ArchiveError::Invalid(
            "unsupported recorded actor pose layout",
        ));
    }
    let geometry = AssetId(reader.bytes()?);
    let appearance = AssetId(reader.bytes()?);
    let metadata: Metadata = serde_json::from_slice(&reader.blob(MAX_METADATA)?)?;
    let instances = reader.packed_limit(
        MAX_ACTOR_RENDER_INSTANCES,
        |words: [u32; ACTOR_GPU_INSTANCE_WORDS]| {
            Some(bytemuck::cast::<
                [u32; ACTOR_GPU_INSTANCE_WORDS],
                ActorGpuInstance,
            >(words))
        },
    )?;
    let previous_bones = reader.packed_limit(
        render_model::actor::MAX_ACTOR_POSE_BONES,
        |words: [u32; BONE_WORDS]| Some(bytemuck::cast::<[u32; BONE_WORDS], Bone>(words)),
    )?;
    let current_bones = reader.packed_limit(
        render_model::actor::MAX_ACTOR_POSE_BONES,
        |words: [u32; BONE_WORDS]| Some(bytemuck::cast::<[u32; BONE_WORDS], Bone>(words)),
    )?;
    let pages = reader.packed_limit(MAX_ACTOR_RENDER_INSTANCES, |words: [u32; 1]| {
        u16::try_from(words[0]).ok()
    })?;
    drop(reader);
    binary::finish(&mut decoded)?;
    binary::finish(&mut decoded.into_inner())?;
    Ok(PendingPose {
        geometry,
        appearance,
        metadata,
        instances,
        previous_bones,
        current_bones,
        pages,
    })
}

impl PendingPose {
    /// Checks every GPU address and raster reference before independently publishing a prepared frame.
    pub(super) fn admit(
        self,
        geometry: &LoadedGeometry,
        appearance: &LoadedAppearance,
    ) -> Result<FrozenActors, ArchiveError> {
        let geometry_identity = self.geometry;
        let appearance_identity = self.appearance;
        let sample = self.admit_rig(geometry, |instance, page| {
            if page == 0 {
                if appearance.skins.resident(instance.texture_layer).is_none() {
                    return Err(ArchiveError::Invalid(
                        "recorded actor skin slot is not resident",
                    ));
                }
            } else {
                let raster = appearance.artwork.pages.get(usize::from(page) - 1).ok_or(
                    ArchiveError::Invalid("recorded actor artwork page is missing"),
                )?;
                if instance.texture_layer >= raster.layers
                    || instance.multitexture_layers.iter().any(|&layer| {
                        layer != u32::MAX && (!raster.multitexture || layer >= raster.layers)
                    })
                {
                    return Err(ArchiveError::Invalid(
                        "recorded actor texture layer exceeds its page",
                    ));
                }
            }
            Ok(())
        })?;
        Ok(FrozenActors {
            geometry_identity,
            appearance_identity,
            sample,
            skins: Arc::clone(&appearance.skins),
            artwork: Arc::clone(&appearance.artwork),
        })
    }

    /// Shared rig admission checks GPU addresses before its draw owner admits each texture selector.
    pub(crate) fn admit_rig(
        self,
        geometry: &LoadedGeometry,
        texture: impl Fn(&ActorGpuInstance, u16) -> Result<(), ArchiveError>,
    ) -> Result<ActorSample, ArchiveError> {
        let Self {
            geometry: _,
            appearance: _,
            metadata,
            instances,
            previous_bones,
            current_bones,
            pages,
        } = self;
        let count = instances.len();
        if count > MAX_ACTOR_RENDER_INSTANCES
            || metadata.instances.len() != count
            || metadata.manifest.len() != count
            || pages.len() != count
            || (std::mem::size_of_val(&*previous_bones) + std::mem::size_of_val(&*current_bones))
                > MAX_ACTOR_BONE_ARENA_BYTES
            || metadata.maximum_vertex_count as usize > MAX_ACTOR_RIG_VERTICES
            || previous_bones
                .iter()
                .chain(&current_bones)
                .flatten()
                .flatten()
                .any(|value| !value.is_finite())
        {
            return Err(ArchiveError::Invalid(
                "invalid recorded actor frame bounds or matrices",
            ));
        }
        let mut identities = BTreeSet::new();
        let manifest = metadata
            .manifest
            .into_iter()
            .map(|entry| entry.restore())
            .collect::<Vec<_>>();
        for (index, ((instance, entry), &page)) in
            instances.iter().zip(&manifest).zip(&pages).enumerate()
        {
            let geometry_index = instance.geometry_id as usize;
            let Some(span) = geometry.spans.get(geometry_index) else {
                return Err(ArchiveError::Invalid("recorded actor geometry is missing"));
            };
            let bone_count = entry.bone_count as usize;
            let previous_end = (entry.previous_bone_base as usize).checked_add(bone_count);
            let current_end = (entry.current_bone_base as usize).checked_add(bone_count);
            let compat = &metadata.instances[index];
            if !entry.identity.is_exact()
                || !identities.insert(entry.identity)
                || entry.route == ActorRigRoute::NoDraw
                || entry.instance_index as usize != index
                || instance.previous_bone_base != entry.previous_bone_base
                || instance.current_bone_base != entry.current_bone_base
                || bone_count == 0
                || bone_count > MAX_RENDER_BONES_PER_ACTOR
                || previous_end.is_none_or(|end| end > previous_bones.len())
                || current_end.is_none_or(|end| end > current_bones.len())
                || geometry.bone_counts[geometry_index] > entry.bone_count
                || span.vertex_count == 0
                || span.vertex_count > metadata.maximum_vertex_count
                || !finite_instance(instance)
                || compat.runtime_id != entry.identity.runtime_id
                || compat
                    .position
                    .iter()
                    .chain([
                        &compat.pitch_radians,
                        &compat.yaw_radians,
                        &compat.head_yaw_radians,
                    ])
                    .any(|value| !value.is_finite())
            {
                return Err(ArchiveError::Invalid(
                    "recorded actor draw references invalid identity, geometry or pose",
                ));
            }
            texture(instance, page)?;
        }
        Ok(ActorSample {
            coverage: metadata.coverage,
            instances: metadata.instances.into(),
            instance_revision: metadata.instance_revision,
            skin_revision: metadata.skin_revision,
            instance_pages: pages.into(),
            rig: ActorRigRenderFrame {
                frame_generation: metadata.frame_generation,
                geometry_revision: metadata.geometry_revision,
                instances: instances.into(),
                previous_bones: previous_bones.into(),
                current_bones: current_bones.into(),
                geometry_vertices: geometry.vertices.clone(),
                geometry_spans: Arc::clone(&geometry.spans),
                manifest: manifest.into(),
                maximum_vertex_count: metadata.maximum_vertex_count,
                rejects: metadata.rejects.restore(),
            },
        })
    }
}

/// Only numerical shader inputs need finiteness checks; packed integer material flags remain exact.
fn finite_instance(instance: &ActorGpuInstance) -> bool {
    let ActorGpuInstance {
        world_from_actor,
        previous_bone_base: _,
        current_bone_base: _,
        geometry_id: _,
        texture_layer: _,
        partial_tick,
        reset_generation: _,
        tint: _,
        overlay_rgba8: _,
        uv_anim,
        light: _,
        multitexture_layers: _,
        material: _,
        dissolve_multiplier,
        light_color_multiplier,
        glint,
    } = instance;
    world_from_actor
        .iter()
        .flatten()
        .chain(uv_anim)
        .chain(glint)
        .chain([partial_tick, dissolve_multiplier, light_color_multiplier])
        .all(|value| value.is_finite())
        && (0.0..=1.0).contains(partial_tick)
}
