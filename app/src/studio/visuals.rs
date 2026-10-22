//! Samples native particles and block models without consuming their authoritative simulation.

use super::Active;
use bevy::prelude::*;
use recording_scene::visuals::{FrozenVisuals, PreparedHand, PreparedParticles, VisualCapture};
use recording_studio::{MediaTime, capture::CaptureSession};
use render_scene::ParticleGpuFrame;
use std::sync::Arc;

pub(super) struct NativeVisuals {
    pub capture: VisualCapture,
    last_frame: u64,
    hand: bool,
    hud: bool,
    atmosphere: super::atmosphere::Assets,
}

/// Records a baseline from immutable native publications, including their explicit admission scope.
pub(super) fn start(world: &World, session: &CaptureSession) -> Result<NativeVisuals, String> {
    let mut capture = VisualCapture::start_with_disk(
        session.directory(),
        session.info().clone(),
        session.disk_budget().clone(),
    )?;
    let hand = session.settings().hand;
    let hud = session.settings().hud;
    let mut atmosphere = super::atmosphere::Assets::default();
    capture.sample(
        MediaTime::ZERO,
        snapshot(world, hand, hud, &mut atmosphere)?,
    )?;
    Ok(NativeVisuals {
        capture,
        last_frame: 0,
        hand,
        hud,
        atmosphere,
    })
}

/// Uses the same video cadence as actors and environment while sharing prepared storage.
pub(super) fn sample(world: &World, active: &mut Active, at: MediaTime) -> Result<(), String> {
    let number = active
        .session
        .settings()
        .fps
        .frame_at(at)
        .map_err(|e| e.to_string())?;
    let Some(visuals) = &mut active.visuals else {
        return Ok(());
    };
    if visuals.last_frame != number {
        visuals.capture.sample(
            at,
            snapshot(world, visuals.hand, visuals.hud, &mut visuals.atmosphere)?,
        )?;
        visuals.last_frame = number;
    }
    Ok(())
}

/// Exhaustive destructuring prevents newly published visual channels being silently omitted.
fn snapshot(
    world: &World,
    hand_enabled: bool,
    hud_enabled: bool,
    atmosphere: &mut super::atmosphere::Assets,
) -> Result<FrozenVisuals, String> {
    let particles = world
        .get_resource::<ParticleGpuFrame>()
        .ok_or("Prepared particles are unavailable for recording.")?;
    let ParticleGpuFrame {
        base,
        atlas_side,
        patch_seq,
        patches,
        opaque,
        blend,
        add,
        cave_hidden: _,
        received,
    } = particles;
    let blocks = world
        .get_resource::<crate::block_entities::system::recording::ReplayBlockEntities>()
        .ok_or("Prepared block models are unavailable for recording.")?;
    Ok(FrozenVisuals {
        nametags: world
            .get_resource::<render_scene::NametagSceneResource>()
            .map(|scene| Arc::new(scene.0.clone())),
        particles: PreparedParticles {
            base: base.clone(),
            atlas_side: *atlas_side,
            patch_seq: *patch_seq,
            patches: Arc::clone(patches),
            opaque: Arc::clone(opaque),
            blend: Arc::clone(blend),
            add: Arc::clone(add),
            received: received.clone(),
        },
        blocks: blocks
            .frame
            .clone()
            .ok_or("Received block models are not prepared yet.")?,
        block_coverage: blocks.coverage,
        items: Some(items(world)?),
        hand_enabled,
        hud_enabled,
        atmosphere: Some(super::atmosphere::snapshot(world, atmosphere)?),
        hud: hud_enabled
            .then(|| {
                let scope = world.get_resource::<render_scene::ui::UiHudCaptureScope>()?;
                if !scope.allowed {
                    return None;
                }
                world
                    .get_resource::<render_scene::ui::UiPreparedHud>()?
                    .latest()
            })
            .flatten(),
        hand: hand_enabled
            .then(|| {
                world
                    .get_resource::<render_scene::HandRigScene>()?
                    .frame
                    .as_ref()
                    .map(|frame| {
                        let render_scene::hand_rig::HandRigFrame {
                            rig,
                            skin,
                            light,
                            fov_radians,
                            revision,
                            item_atlases,
                            item_glint,
                        } = frame;
                        PreparedHand {
                            rig: rig.clone(),
                            skin: skin.clone(),
                            light: *light,
                            fov_radians: *fov_radians,
                            revision: *revision,
                            item_atlases: item_atlases.clone(),
                            item_glint: item_glint.clone(),
                        }
                    })
            })
            .flatten(),
    })
}

/// Freezes the exact native model slots, textures and visibility transitions without live authority.
fn items(world: &World) -> Result<recording_scene::visuals::PreparedItems, String> {
    let terrain = world
        .get_resource::<render_scene::dropped_item::recording::ItemReplayVisibility>()
        .and_then(|v| v.latest());
    let replay = world
        .get_resource::<client_presentation::dropped_items::recording::ReplayItems>()
        .ok_or("Received item preparation is unavailable.")?;
    let render_scene::DroppedItemScene {
        models_revision: _,
        models,
        instances,
        terrain_session_id,
        terrain_instances,
        dynamic,
        daylight,
        glint,
        glint_scroll,
    } = replay
        .scene
        .as_ref()
        .ok_or("Received items are not prepared yet.")?;
    let terrain = terrain.filter(|t| t.session_id == *terrain_session_id);
    let client_presentation::dropped_items::recording::Admission {
        model_failures,
        instance_limit,
        rope_limit,
    } = replay.admission;
    Ok(recording_scene::visuals::PreparedItems {
        models: Arc::clone(models),
        instances: Arc::clone(instances),
        terrain_session_id: *terrain_session_id,
        terrain_instances: Arc::clone(terrain_instances),
        dynamic: Arc::clone(dynamic),
        ropes: Arc::clone(&replay.ropes),
        terrain: terrain.clone(),
        daylight: *daylight,
        glint: glint.clone(),
        glint_scroll: *glint_scroll,
        coverage: recording_scene::visuals::ItemCoverage {
            source_seconds: replay.source_seconds,
            model_failures,
            instance_limit,
            rope_limit,
            terrain_pending: terrain.as_ref().is_none_or(|t| !t.complete),
        },
    })
}

/// Only gameplay HUD and chat can enter the opted-in recording; menus and other screens clear it.
pub(super) fn scope(world: &mut World, enabled: bool) {
    use client_ui::ui_runtime::scene_stack::{Scene, SceneHost};
    let allowed = enabled
        && (|| {
            let player = world.get_resource::<client_state::PlayerRuntime>()?;
            let ui = world.get_resource::<client_shell::ui_resources::UiRuntimeResource>()?;
            let menu = world.get_resource::<client_shell::menu::MenuRuntime>()?;
            let stack = ui.scenes(player, SceneHost::of(Some(menu), ui));
            let scenes = stack.scenes();
            Some(
                scenes.iter().any(|scene| scene.key == Scene::Gameplay)
                    && scenes.iter().all(|scene| {
                        matches!(
                            scene.key,
                            Scene::Gameplay | Scene::Crosshair | Scene::Hud | Scene::Chat
                        )
                    }),
            )
        })()
        .unwrap_or(false);
    world.insert_resource(render_scene::ui::UiHudCaptureScope { enabled, allowed });
    if !allowed && let Some(hud) = world.get_resource::<render_scene::ui::UiPreparedHud>() {
        hud.publish(None);
    }
}
