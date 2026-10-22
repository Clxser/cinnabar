//! Samples published weather and camera effects while retaining immutable raster epochs.

use bevy::prelude::*;
use recording_scene::visuals::atmosphere::{
    AtmosphereAssets, AtmosphereVisuals, OverlayKind, OverlayLayer, RecordedFire,
};
use render_scene::{
    AtmosphereViewInputs, CloudVisibility, LightningScene, PrecipitationScene, ScreenOverlayKind,
    ScreenOverlayScene, SkyboxSettings, WeatherTextureAssets,
};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Assets {
    key: Option<([u8; 32], u64, u64)>,
    prepared: Option<Arc<AtmosphereAssets>>,
}

/// Only texture replacement copies the legacy vector-backed camera masks or fire timeline.
pub(super) fn snapshot(world: &World, cache: &mut Assets) -> Result<AtmosphereVisuals, String> {
    let weather = world
        .get_resource::<WeatherTextureAssets>()
        .ok_or("Prepared weather textures are unavailable.")?;
    let overlays = world
        .get_resource::<ScreenOverlayScene>()
        .ok_or("Prepared camera effects are unavailable.")?;
    let key = (
        weather.identity(),
        overlays.textures_revision,
        overlays.fire_revision,
    );
    if cache.key != Some(key) {
        let fire = overlays.fire.as_ref().map(|fire| {
            let (timeline, ticks_per_frame, blend) = fire.recording_animation();
            Arc::new(RecordedFire {
                side: fire.side,
                frames: fire.frames,
                pixels: Arc::from(&fire.pixels[..]),
                timeline: Arc::from(timeline),
                ticks_per_frame,
                blend,
            })
        });
        cache.prepared = Some(Arc::new(AtmosphereAssets {
            identity: weather.identity(),
            weather: weather.textures().cloned(),
            end_sky: weather.shared_end_sky_override(),
            pumpkin: overlays
                .textures
                .as_ref()
                .and_then(|t| t.pumpkin())
                .map(Arc::from),
            spyglass: overlays
                .textures
                .as_ref()
                .and_then(|t| t.spyglass())
                .map(Arc::from),
            mask_side: render_scene::SCREEN_OVERLAY_TEXTURE_SIDE,
            fire,
        }));
        cache.key = Some(key);
    }
    let precipitation = world
        .get_resource::<PrecipitationScene>()
        .ok_or("Prepared precipitation is unavailable.")?;
    let view = world
        .get_resource::<AtmosphereViewInputs>()
        .ok_or("Prepared atmosphere view inputs are unavailable.")?;
    let skybox = world
        .get_resource::<SkyboxSettings>()
        .copied()
        .unwrap_or_default();
    let lightning = world
        .get_resource::<LightningScene>()
        .ok_or("Prepared lightning is unavailable.")?;
    let lightning = lightning
        .records
        .iter()
        .map(|record| {
            let &render_scene::BoltRecord {
                start,
                half_width,
                end,
                intensity,
            } = record;
            [
                start[0], start[1], start[2], half_width, end[0], end[1], end[2], intensity,
            ]
        })
        .collect::<Vec<_>>()
        .into();
    Ok(AtmosphereVisuals {
        assets: Arc::clone(
            cache
                .prepared
                .as_ref()
                .ok_or("Prepared weather epoch is unavailable.")?,
        ),
        precipitation: Arc::from(&precipitation.layers[..]),
        forward_offset: precipitation.forward_offset,
        occlusion_origin: precipitation.occlusion.origin,
        occlusion_heights: Arc::from(&precipitation.occlusion.heights[..]),
        occlusion_generation: precipitation.occlusion_generation,
        lightning,
        overlays: overlays
            .layers
            .iter()
            .map(|layer| OverlayLayer {
                kind: kind(layer.kind),
                rgb: layer.rgb,
                alpha: layer.alpha,
            })
            .collect::<Vec<_>>()
            .into(),
        overlay_clock: overlays.clock_seconds,
        fire_projection: overlays.fire_projection,
        portal_from_clip: overlays.portal_from_clip.to_cols_array_2d(),
        fog_weather_level: view.fog_weather_level,
        current_rain_level: view.current_rain_level,
        clouds: world.get_resource::<CloudVisibility>().is_none_or(|v| v.0),
        skybox_visible: skybox.visible,
        skybox_native_offset: skybox.native_vertical_offset,
    })
}

/// Exhaustive owner matching makes a new camera-effect layer require an explicit recording schema choice.
fn kind(kind: ScreenOverlayKind) -> OverlayKind {
    match kind {
        ScreenOverlayKind::Flat => OverlayKind::Flat,
        ScreenOverlayKind::PowderSnow => OverlayKind::PowderSnow,
        ScreenOverlayKind::Fire => OverlayKind::Fire,
        ScreenOverlayKind::Portal => OverlayKind::Portal,
        ScreenOverlayKind::PumpkinBlur => OverlayKind::PumpkinBlur,
        ScreenOverlayKind::SpyglassScope => OverlayKind::SpyglassScope,
    }
}
