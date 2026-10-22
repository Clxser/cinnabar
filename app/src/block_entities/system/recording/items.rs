//! Records item contents only after the independent block scene has resolved their placements.

use super::*;
use bevy::ecs::system::SystemParam;

#[derive(SystemParam)]
pub(in crate::block_entities::system) struct Items<'w, 's> {
    replay: Option<ResMut<'w, client_presentation::dropped_items::recording::ReplayItems>>,
    icons: Option<Res<'w, client_shell::ui_resources::UiPresentationResource>>,
    textures: Option<Res<'w, render_scene::ChunkTextureAssets>>,
    source: Option<Res<'w, render_scene::DroppedItemScene>>,
    device: Option<Res<'w, bevy::render::renderer::RenderDevice>>,
    camera: Query<'w, 's, &'static Transform, With<client_presentation::camera::FlyCamera>>,
}

impl Items<'_, '_> {
    /// Uses the native source camera for original ropes and lifetime inputs, with no frustum rejection.
    pub(super) fn prepare(
        &mut self,
        stream: Option<&chunk_pipeline::WorldStream>,
        collisions: &PhysicsCollisionRegistries,
        partial_tick: f32,
        now: f64,
        placements: &[StaticItemPlacement],
    ) {
        let (Some(stream), Some(icons), Some(source), Ok(camera)) = (
            stream,
            self.icons.as_ref(),
            self.source.as_ref(),
            self.camera.single(),
        ) else {
            return;
        };
        let (yaw, _, _) = camera.rotation.to_euler(bevy::math::EulerRot::YXZ);
        let Some(replay) = self.replay.as_deref_mut() else {
            return;
        };
        replay.prepare(
            &icons.0,
            stream,
            Some(collisions),
            (
                camera.translation.to_array(),
                (180.0 - yaw.to_degrees()).rem_euclid(360.0),
            ),
            partial_tick,
            now,
            self.textures.as_deref(),
            placements,
            self.device.as_deref(),
            source,
        );
    }
}
