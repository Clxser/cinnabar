//! Restores prepared weather and source-camera effects in the independent replay world.

use super::ReplayRenderer;
use bevy::prelude::*;
use recording_scene::{playback::SceneFrame, visuals::atmosphere::OverlayKind};
use render_scene::{
    ScreenOverlayKind, ScreenOverlayLayer, ScreenOverlayScene, WeatherTextureAssets,
};
use std::sync::Arc;

impl ReplayRenderer {
    /// Scene-local weather texture epochs never reuse a live session's GPU cache identity.
    pub(super) fn apply_atmosphere_visuals(
        &mut self,
        frame: &SceneFrame,
        transform: Transform,
        source_view: bool,
    ) -> Result<(), String> {
        let Some(prepared) = frame
            .visuals
            .as_ref()
            .and_then(|visuals| visuals.atmosphere.as_ref())
        else {
            let world = self.app.world_mut();
            world.insert_resource(render_scene::PrecipitationScene::default());
            world.insert_resource(render_scene::LightningScene::default());
            world.insert_resource(WeatherTextureAssets::default());
            world.insert_resource(render_scene::AtmosphereViewInputs::default());
            world.insert_resource(render_scene::CloudVisibility::default());
            world.insert_resource(render_scene::SkyboxSettings::default());
            world.resource_mut::<ScreenOverlayScene>().layers.clear();
            self.atmosphere_visual_assets = None;
            return Ok(());
        };
        let assets = &prepared.assets;
        let changed = self
            .atmosphere_visual_assets
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, assets));
        let world = self.app.world_mut();
        if changed {
            let mut weather = assets
                .weather
                .as_ref()
                .map_or_else(WeatherTextureAssets::default, |textures| {
                    WeatherTextureAssets::new(Arc::clone(textures), assets.identity)
                });
            if let Some(end_sky) = &assets.end_sky {
                weather = weather.with_end_sky((**end_sky).clone(), assets.identity);
            }
            world.insert_resource(weather);
            if (assets.pumpkin.is_some() || assets.spyglass.is_some())
                && assets.mask_side != render_scene::SCREEN_OVERLAY_TEXTURE_SIDE
            {
                return Err("The recorded camera-mask size is unsupported.".into());
            }
            let masks = if assets.pumpkin.is_some() || assets.spyglass.is_some() {
                Some(Arc::new(
                    render_scene::ScreenOverlayTextures::from_layers(
                        assets.pumpkin.as_ref().map(|p| p.to_vec()),
                        assets.spyglass.as_ref().map(|p| p.to_vec()),
                    )
                    .ok_or("The recorded camera masks were rejected.")?,
                ))
            } else {
                None
            };
            let fire = assets
                .fire
                .as_ref()
                .map(|fire| {
                    render_scene::ScreenFireTexture::from_recording(
                        fire.side,
                        fire.pixels.to_vec(),
                        fire.frames,
                        fire.timeline.to_vec(),
                        fire.ticks_per_frame,
                        fire.blend,
                    )
                    .map(Arc::new)
                    .ok_or("The recorded camera fire was rejected.")
                })
                .transpose()?;
            let mut overlay = world.resource_mut::<ScreenOverlayScene>();
            overlay.set_textures(masks);
            overlay.set_fire_texture(fire);
            self.atmosphere_visual_assets = Some(Arc::clone(assets));
        }
        let mut grid = render_model::OcclusionGrid::default();
        grid.origin = prepared.occlusion_origin;
        grid.heights = prepared.occlusion_heights.to_vec();
        world.insert_resource(render_scene::PrecipitationScene {
            layers: prepared.precipitation.to_vec(),
            forward_offset: prepared.forward_offset,
            occlusion: Arc::new(grid),
            occlusion_generation: prepared.occlusion_generation,
        });
        world.insert_resource(render_scene::LightningScene {
            records: prepared
                .lightning
                .iter()
                .map(|r| render_scene::BoltRecord {
                    start: [r[0], r[1], r[2]],
                    half_width: r[3],
                    end: [r[4], r[5], r[6]],
                    intensity: r[7],
                })
                .collect(),
        });
        world.insert_resource(render_scene::AtmosphereViewInputs {
            forward: transform.forward().to_array(),
            fog_weather_level: prepared.fog_weather_level,
            current_rain_level: prepared.current_rain_level,
        });
        world.insert_resource(render_scene::CloudVisibility(prepared.clouds));
        world.insert_resource(render_scene::SkyboxSettings {
            native_vertical_offset: prepared.skybox_native_offset,
            visible: prepared.skybox_visible,
        });
        let mut overlays = world.resource_mut::<ScreenOverlayScene>();
        overlays.set_layers(
            prepared
                .overlays
                .iter()
                .filter(|_| source_view)
                .map(|layer| ScreenOverlayLayer {
                    kind: kind(layer.kind),
                    rgb: layer.rgb,
                    alpha: layer.alpha,
                }),
            prepared.overlay_clock,
        );
        overlays.fire_projection = prepared.fire_projection;
        overlays.portal_from_clip = Mat4::from_cols_array_2d(&prepared.portal_from_clip);
        Ok(())
    }
}

/// Every portable source-camera effect maps to one explicit native renderer route.
fn kind(kind: OverlayKind) -> ScreenOverlayKind {
    match kind {
        OverlayKind::Flat => ScreenOverlayKind::Flat,
        OverlayKind::PowderSnow => ScreenOverlayKind::PowderSnow,
        OverlayKind::Fire => ScreenOverlayKind::Fire,
        OverlayKind::Portal => ScreenOverlayKind::Portal,
        OverlayKind::PumpkinBlur => ScreenOverlayKind::PumpkinBlur,
        OverlayKind::SpyglassScope => ScreenOverlayKind::SpyglassScope,
    }
}
