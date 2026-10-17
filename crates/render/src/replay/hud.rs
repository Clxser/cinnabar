//! Source-camera HUD restoration uses independent UI resources and a recorded shader clock.

use super::ReplayRenderer;
use bevy::{prelude::*, render::RenderApp};
use recording_scene::playback::SceneFrame;
use render_model::{UiRenderScene, UiRenderStats};

impl ReplayRenderer {
    /// Admits a scaled copy of prepared screen geometry without consulting a live UI authority.
    pub(super) fn apply_hud(
        &mut self,
        frame: &SceneFrame,
        recorded_view: bool,
    ) -> Result<(), String> {
        let hud = frame
            .visuals
            .as_ref()
            .and_then(|visuals| visuals.hud.as_deref())
            .filter(|_| recorded_view);
        let identity = hud.map(|hud| hud.input.textures.static_identity());
        if identity != self.hud_palette {
            crate::ui_render::reset_replay(self.app.sub_app_mut(RenderApp).world_mut());
            self.hud_palette = identity;
        }
        let world = self.app.world_mut();
        let Some(hud) = hud else {
            world.insert_resource(render_scene::UiRenderSceneResource::default());
            world.insert_resource(render_scene::ui::RecordedUiTime(None));
            return Ok(());
        };
        let mut input = (*hud.input).clone();
        let source = input.viewport_size;
        let ratio = [
            self.size[0] as f32 / source[0] as f32,
            self.size[1] as f32 / source[1] as f32,
        ];
        input.revision = self.generation;
        input.viewport_size = self.size;
        input.vertices = input
            .vertices
            .iter()
            .map(|vertex| {
                let mut vertex = *vertex;
                vertex.position[0] *= ratio[0];
                vertex.position[1] *= ratio[1];
                vertex
            })
            .collect::<Vec<_>>()
            .into();
        input.safe_area =
            std::array::from_fn(|index| (input.safe_area[index] as f32 * ratio[index % 2]) as u32);
        input.batches = input
            .batches
            .iter()
            .map(|batch| {
                let mut batch = *batch;
                let scissor = batch.scissor;
                let x = (scissor.x as f32 * ratio[0]).floor() as u32;
                let y = (scissor.y as f32 * ratio[1]).floor() as u32;
                let right = ((scissor.x + scissor.width) as f32 * ratio[0]).ceil() as u32;
                let bottom = ((scissor.y + scissor.height) as f32 * ratio[1]).ceil() as u32;
                batch.scissor = render_model::UiScissor::new(
                    x,
                    y,
                    right.min(self.size[0]).saturating_sub(x),
                    bottom.min(self.size[1]).saturating_sub(y),
                );
                batch
            })
            .collect::<Vec<_>>()
            .into();
        let mut scene = UiRenderScene::default();
        scene
            .publish(input, &UiRenderStats::default())
            .map_err(|error| error.to_string())?;
        world.insert_resource(render_scene::UiRenderSceneResource(scene));
        world.insert_resource(render_scene::ui::RecordedUiTime(Some(hud.time_seconds)));
        world.insert_resource(render_scene::UiGlintSettings {
            strength: hud.glint_strength,
            speed: 1.0,
        });
        world.insert_resource(render_scene::UiItemGlintTexture(hud.glint.clone()));
        Ok(())
    }
}
