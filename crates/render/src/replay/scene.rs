//! Publishes frozen scene contracts into the replay app, never the live main or render world.

use std::sync::Arc;

use bevy::prelude::*;
use meshing::ChunkBiomeTintIdentity;
use recording_scene::{palette::FrozenTint, playback::SceneFrame};
use recording_studio::camera::CameraPose;
use render_scene::{
    ActorRenderFrame, ActorRenderInstance, AtmosphereFrame, AtmosphereTextureAssets, BiomeTint,
    ChunkAnimationClock, ChunkBiomeTints, ChunkRenderInstance, ChunkTextureAssets,
    ChunkUploadPriority, WorldFullbright, WorldLighting,
};

use super::{RecordedClock, ReplayRenderer};

impl ReplayRenderer {
    /// The selected global epoch is independent of older resident chunk bindings.
    pub(super) fn apply_scene(
        &mut self,
        frame: &SceneFrame,
        recorded_view: bool,
        hand_visible: bool,
    ) -> Result<(), String> {
        let patch = &frame.terrain;
        let world = self.app.world_mut();
        let dimension = frame.environment.state.view.map(|view| view.dimension);
        if patch.reset {
            for entity in self.chunks.values() {
                world.despawn(*entity);
            }
            self.chunks.clear();
        }
        let selection = &patch.active_palette;
        let palette = &selection.palette;
        if self.palette != Some(selection.id) {
            world.insert_resource(ChunkTextureAssets::new(Arc::clone(&palette.assets)));
            let tints: Arc<[BiomeTint]> = palette
                .tints
                .iter()
                .map(restore_tint)
                .collect::<Vec<_>>()
                .into();
            world.insert_resource(ChunkBiomeTints::with_identity(
                tints,
                ChunkBiomeTintIdentity::new(
                    palette.binding.tint_stream,
                    palette.binding.tint_revision,
                ),
            ));
            self.palette = Some(selection.id);
            self.terrain_dirty = true;
        }
        for key in &patch.removed {
            if let Some(entity) = self.chunks.remove(key) {
                world.despawn(entity);
            }
        }
        self.terrain_dirty |= patch.reset || !patch.upserts.is_empty() || !patch.removed.is_empty();
        for chunk in &patch.upserts {
            if dimension.is_some_and(|dimension| chunk.binding.key.dimension != dimension) {
                if let Some(entity) = self.chunks.remove(&chunk.binding.key) {
                    world.despawn(entity);
                }
                continue;
            }
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("Replay render generation overflowed.")?;
            let key = chunk.binding.key.key().map_err(|error| error.to_string())?;
            let origin = chunk
                .binding
                .key
                .origin()
                .map_err(|error| error.to_string())?;
            let mesh = &chunk.mesh;
            let depth_liquid_start = mesh
                .liquids
                .iter()
                .position(|quad| quad.is_depth_writing())
                .map(|index| index as u32);
            let instance = ChunkRenderInstance {
                key,
                cube_quads: Arc::clone(&mesh.cubes),
                cube_lighting: Arc::clone(&mesh.cube_lighting),
                cube_layout: mesh.cube_layout,
                model_refs: Arc::clone(&mesh.models),
                model_lighting: Arc::clone(&mesh.model_lighting),
                model_draw_refs: Arc::clone(&mesh.model_draws),
                transparent_model_draw_refs: Arc::clone(&mesh.transparent_model_draws),
                liquid_quads: Arc::clone(&mesh.liquids),
                liquid_lighting: Arc::clone(&mesh.liquid_lighting),
                has_depth_liquid: depth_liquid_start.is_some(),
                has_transparent_liquid: mesh
                    .liquids
                    .first()
                    .is_some_and(|quad| !quad.is_depth_writing()),
                depth_liquid_start,
                biome: mesh.biome.clone(),
                tint_identity: ChunkBiomeTintIdentity::new(
                    chunk.palette.binding.tint_stream,
                    chunk.palette.binding.tint_revision,
                ),
                generation: self.generation,
                priority: ChunkUploadPriority::urgent(),
                token: None,
                publication_permit: None,
                origin,
            };
            let bounds = crate::chunk::bounds::aabb(!mesh.models.is_empty());
            if let Some(entity) = self.chunks.get(&chunk.binding.key) {
                world.entity_mut(*entity).insert((instance, bounds));
            } else {
                let entity = world
                    .spawn((
                        instance,
                        bounds,
                        Visibility::default(),
                        Transform::from_translation(Vec3::from_array(
                            origin.map(|axis| axis as f32),
                        )),
                    ))
                    .id();
                self.chunks.insert(chunk.binding.key, entity);
            }
        }
        let environment = &frame.environment;
        if self.environment != Some(environment.assets_identity) {
            let mut textures = AtmosphereTextureAssets::new(
                Arc::clone(&environment.atmosphere),
                environment.assets_identity.0,
            );
            if let Some(skyboxes) = &environment.skyboxes {
                textures =
                    textures.with_skyboxes((**skyboxes).clone(), environment.assets_identity.0);
            }
            world.insert_resource(textures);
            self.environment = Some(environment.assets_identity);
        }
        world.insert_resource(
            AtmosphereFrame::from_snapshot(environment.state.atmosphere)
                .ok_or("The recorded atmosphere was rejected.")?,
        );
        world.insert_resource(WorldLighting(environment.state.lightmap.restore()));
        world.insert_resource(WorldFullbright(environment.state.fullbright));
        world.resource_mut::<RecordedClock>().0 = ChunkAnimationClock::from_parts(
            environment.state.animation_tick,
            environment.state.animation_partial,
        );
        let actors = &frame.actors;
        let hide_local = recorded_view
            && frame
                .environment
                .state
                .view
                .is_some_and(|view| !view.local_body_visible);
        let mut rig = actors.sample.rig.clone();
        let keep = |index: usize| {
            !hide_local || frame.local_actor != rig.manifest[index].identity.runtime_id
        };
        let selected = (0..rig.instances.len())
            .filter(|&index| keep(index))
            .collect::<Vec<_>>();
        let instances = actors
            .sample
            .instances
            .iter()
            .enumerate()
            .filter(|(index, _)| selected.binary_search(index).is_ok())
            .map(|(_, instance)| ActorRenderInstance {
                runtime_id: instance.runtime_id,
                position: instance.position,
                pitch_radians: instance.pitch_radians,
                yaw_radians: instance.yaw_radians,
                head_yaw_radians: instance.head_yaw_radians,
                skin_layer: instance.skin_layer,
            })
            .collect::<Vec<_>>()
            .into();
        let instance_pages = selected
            .iter()
            .map(|&index| actors.sample.instance_pages[index])
            .collect::<Vec<_>>()
            .into();
        rig.instances = selected
            .iter()
            .map(|&index| rig.instances[index])
            .collect::<Vec<_>>()
            .into();
        rig.manifest = selected
            .iter()
            .enumerate()
            .map(|(next, &index)| {
                let mut entry = rig.manifest[index].clone();
                entry.instance_index = next as u32;
                entry
            })
            .collect::<Vec<_>>()
            .into();
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("Replay actor generation overflowed.")?;
        rig.frame_generation = self.generation;
        world.insert_resource(ActorRenderFrame {
            instances,
            skins: Arc::clone(&actors.skins),
            instance_revision: actors.sample.instance_revision,
            skin_revision: actors.sample.skin_revision,
            rig,
            artwork: Arc::clone(&actors.artwork),
            instance_pages,
        });
        let visuals = frame.visuals.clone().unwrap_or_default();
        let hand = visuals
            .hand
            .filter(|_| recorded_view && hand_visible)
            .map(|hand| render_scene::hand_rig::HandRigFrame {
                rig: hand.rig,
                skin: hand.skin,
                light: hand.light,
                fov_radians: hand.fov_radians,
                revision: hand.revision,
                item_atlases: hand.item_atlases,
                item_glint: hand.item_glint,
            });
        world.insert_resource(render_scene::HandRigScene { frame: hand });
        let particles = visuals.particles;
        world.insert_resource(render_scene::ParticleGpuFrame {
            base: particles.base,
            atlas_side: particles.atlas_side,
            patch_seq: particles.patch_seq,
            patches: particles.patches,
            opaque: particles.opaque,
            blend: particles.blend,
            add: particles.add,
            cave_hidden: 0,
            received: particles.received,
        });
        world.insert_resource(render_scene::BlockEntityFrameResource(visuals.blocks));
        Ok(())
    }
}

/// Bedrock yaw zero faces +Z; the native camera's local forward is -Z.
pub(super) fn camera_transform(pose: CameraPose) -> Result<Transform, String> {
    let translation = Vec3::from_array(pose.position.map(|axis| axis as f32));
    if !translation.is_finite() {
        return Err("The replay camera lies outside native render coordinates.".into());
    }
    let rotation = Quat::from_euler(
        EulerRot::YXZ,
        std::f32::consts::PI - pose.yaw.rem_euclid(360.0).to_radians() as f32,
        -pose.pitch.to_radians() as f32,
        pose.roll.rem_euclid(360.0).to_radians() as f32,
    );
    Ok(Transform {
        translation,
        rotation,
        ..default()
    })
}

/// Exhaustive channels keep replay tint restoration aligned with native material shading.
fn restore_tint(tint: &FrozenTint) -> BiomeTint {
    let &FrozenTint {
        grass,
        foliage,
        birch,
        evergreen,
        dry_foliage,
        seasonal_foliage,
        water,
        water_opacity,
        flags,
    } = tint;
    BiomeTint {
        grass,
        foliage,
        birch,
        evergreen,
        dry_foliage,
        seasonal_foliage,
        water,
        water_opacity,
        flags,
    }
}
