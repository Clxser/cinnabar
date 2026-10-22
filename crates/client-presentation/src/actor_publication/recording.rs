//! Recording-only actor preparation uses received rigs before the gameplay camera selects draws.

use super::*;
use crate::presentation::actors::select_actor_presentations_for_view;
use bevy::prelude::Resource;
use render_scene::ActorRenderFrame;

#[derive(Resource, Default)]
pub struct ReplayActors {
    pub enabled: bool,
    pub frame: Option<ActorRenderFrame>,
    requested: bool,
    scratch: ActorFrameState,
    scene: ActorRenderScene,
    source_catalog: Option<render_model::ActorRigCatalogIdentity>,
    source_artwork: Option<render_model::ActorArtworkPages>,
    /// Most recent admitted recording-only preparation; excludes ordinary gameplay preparation.
    pub prepare_micros: u64,
}

impl ReplayActors {
    /// Releases prepared recordings and controller caches when capture admission ends.
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled && !enabled {
            self.frame = None;
            self.requested = false;
            self.scratch = ActorFrameState::default();
            self.scene = ActorRenderScene::default();
            self.source_catalog = None;
            self.source_artwork = None;
            self.prepare_micros = 0;
        }
        self.enabled = enabled;
    }

    /// Admits one extra native publication at the capture owner's selected cadence.
    pub fn request_frame(&mut self) {
        self.requested = self.enabled;
    }
}

/// Samples the same admitted actors, artwork and animation clock with an independent body view.
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare<'a>(
    replay: &mut ReplayActors,
    stream: Option<&'a WorldStream>,
    input: &ActorFrameInput,
    mut view: bodies::PoseView,
    visibility: Option<crate::local_player::FrozenLocalAvatarVisibility>,
    artwork: &render_model::ActorArtworkPages,
    entity_assets: Option<&assets::RuntimeEntityAssets>,
    pack: Option<&assets::SessionEntityPack>,
    equipment: Option<&mut EquipmentRuntime>,
    skin_rigs: &mut crate::presentation::skin_rig::SkinRigCache,
    scene: &mut ActorRenderScene,
    hand: &mut ActorRigFrameBuilder,
    glint: Option<&render_scene::UiGlintSettings>,
    profiler: Option<&RuntimeStageProfiler>,
    sampled_frame: Option<actor_animation::ActorRenderFrame<'a>>,
) {
    if !replay.enabled || !std::mem::take(&mut replay.requested) {
        return;
    }
    let started = std::time::Instant::now();
    // The hand keeps the ordinary first-person sample; replay bodies use their world-space pose.
    view.first_person = false;
    let session = stream.map(|stream| stream.authority().actor_session_id());
    if replay.scratch.published_session != session {
        replay.scratch = ActorFrameState::default();
        replay.scratch.published_session = session;
        replay.source_catalog = None;
        replay.source_artwork = None;
        replay.scene = ActorRenderScene::default();
    }
    if replay.source_catalog != Some(scene.catalog_identity()) {
        replay.scene.copy_catalog_from(scene);
        replay.source_catalog = Some(scene.catalog_identity());
    }
    if replay
        .source_artwork
        .as_ref()
        .is_none_or(|old| !old.shares_storage_with(artwork))
    {
        replay.scene.configure_artwork(artwork.clone());
        replay.source_artwork = Some(artwork.clone());
    }
    let scratch = &mut replay.scratch;
    scratch.poses.begin_frame();
    scratch.java_poses.begin_frame();
    scratch.layer_poses.begin_frame();
    let mut equipment = equipment;
    let mut geometries = Vec::new();
    let mut bodies = bodies::sample(
        stream,
        input,
        (view, None),
        artwork,
        bodies::BodyCaches {
            coverage: &[],
            poses: &mut scratch.poses,
            java_poses: &mut scratch.java_poses,
            native_pose: &mut scratch.java_hand.native_pose,
            skin_rigs,
            equipment: equipment.as_deref_mut(),
            scratch: &mut scratch.body_scratch,
        },
        |_, _, _, _| false,
        &mut geometries,
        sampled_frame.map(actor_animation::ActorRenderFrame::into_additional_view),
    );
    let (_, local) = bodies::place_local(
        &mut bodies,
        stream,
        visibility,
        view.partial_tick,
        skin_rigs,
        equipment.as_deref_mut(),
        &mut geometries,
    );
    let mut batch = select_actor_presentations_for_view(
        bodies.local_runtime_id,
        true,
        local,
        bodies.remotes,
        None,
    );
    scratch.paintings.refresh(entity_assets, pack, artwork);
    if let Some(stream) = stream {
        scratch
            .paintings
            .append(stream.authority(), view.partial_tick, None, &mut batch);
        crate::presentation::actors::light_bodies(&mut batch, stream);
    }
    layers::attach(
        &mut batch,
        stream,
        bodies.render_frame.as_mut(),
        layers::LayerPoses {
            local_runtime_id: bodies.local_runtime_id,
            local_equipment: &input.local_equipment,
            java_posed: &bodies.java_posed,
            combat_visuals: view.combat_visuals,
            local_emote_pose: bodies.local_emote_pose.as_ref(),
            remote_emote_poses: &bodies.remote_emote_poses,
            native_body_sampled: bodies.native_body_sampled,
            local_coverage: false,
            remote_skin_layers: &bodies.remote_skin_layers,
            native_pose: &scratch.java_hand.native_pose,
        },
        layers::LayerCaches {
            layer_poses: &mut scratch.layer_poses,
            equipment: equipment.as_deref_mut(),
            cape_state: &mut scratch.cape_state,
            skin_layers: &mut scratch.skin_layers,
            skin_rigs,
        },
        artwork,
        entity_assets,
        &mut replay.scene,
        (view.partial_tick, 0.0, glint),
        &mut geometries,
    );
    layers::apply_style_and_visibility(&mut batch, stream, None);
    scratch
        .body_scratch
        .recycle(bodies.java_posed, bodies.remote_skin_layers);
    if let Some(equipment) = equipment {
        geometries.extend(equipment.take_pending_geometries());
    }
    // New skin/item geometry remains registered for subsequent live frames. Recording draws and
    // artwork use their own scene, so they cannot reset the live rig or churn its GPU residency.
    register_geometries(hand, scene, geometries, profiler);
    if replay.source_catalog != Some(scene.catalog_identity()) {
        replay.scene.copy_catalog_from(scene);
        replay.source_catalog = Some(scene.catalog_identity());
    }
    replay.frame = Some(
        crate::presentation::actors::update_actor_rig_scene(
            &mut replay.scene,
            view.partial_tick,
            batch,
        )
        .clone(),
    );
    replay.prepare_micros = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
}

#[cfg(test)]
mod tests;
