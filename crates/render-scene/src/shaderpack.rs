//! Main-world state of the experimental Java shader-pack render mode: which pack the player
//! selected, how loading went, the per-camera opt-in, and the game state packs read through
//! their standard uniforms. The renderer behind it compiles only with the `shaderpacks`
//! feature; without it these types stay unused.

use std::path::PathBuf;

use bevy::{
    ecs::system::SystemParam,
    prelude::{Component, Entity, Query, Res, Resource, With},
    render::{extract_component::ExtractComponent, extract_resource::ExtractResource},
};

/// The pack the settings ask for. The render host loads it on a worker whenever `pack`
/// changes and reports progress through [`ShaderPackStatus`].
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct ShaderPackRequest {
    /// Pack folder or zip; `None` unloads the current pack.
    pub pack: Option<PathBuf>,
    /// Private directory for cached translations; `None` disables the cache.
    pub cache_dir: Option<PathBuf>,
}

/// Load state of the requested pack, written by the render host.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub enum ShaderPackStatus {
    /// No pack requested.
    #[default]
    Idle,
    /// Translation is running for this pack.
    Loading(PathBuf),
    /// The pack is translated and its programs are ready to bind.
    Ready(PathBuf),
    /// The pack could not be used; cameras stay on Vanilla.
    Failed { pack: PathBuf, reason: String },
}

impl ShaderPackStatus {
    /// Whether cameras may opt into the loaded pack.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }
}

/// Per-camera opt-in: the camera's world passes run through the loaded pack.
#[derive(Component, ExtractComponent, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[extract_app(bevy::render::RenderApp)]
#[require(bevy::render::view::Msaa::Off)]
pub struct ShaderPackRendering;

/// Game state behind the standard Iris uniforms that the render world cannot derive from
/// the view. Published each frame by the app; camera matrices, positions, screen size and
/// frame timing come from the extracted view and render-world clock instead.
#[derive(Resource, ExtractResource, Clone, Copy, Debug, PartialEq)]
#[extract_app(bevy::render::RenderApp)]
pub struct ShaderPackFrameInputs {
    /// Time of day in ticks, `0..24000` (`worldTime`).
    pub world_time: i32,
    /// Whole days elapsed (`worldDay`).
    pub world_day: i32,
    /// Moon phase `0..8` (`moonPhase`).
    pub moon_phase: i32,
    /// Celestial angle as a fraction of a full turn, `0..1`, where 0 is noon (Java's
    /// `getTimeOfDay`); drives `sunAngle`, `shadowAngle` and the celestial positions.
    pub celestial_angle: f32,
    /// Rain level `0..1` (`rainStrength`).
    pub rain_strength: f32,
    /// Thunder level `0..1` (`thunderStrength`).
    pub thunder_strength: f32,
    /// Block and sky light at the eye, each `0..240` in steps of 16 (`eyeBrightness`).
    pub eye_brightness: [i32; 2],
    /// 0 air, 1 water, 2 lava, 3 powder snow (`isEyeInWater`).
    pub eye_medium: i32,
    /// Linear sky colour at the zenith (`skyColor`).
    pub sky_color: [f32; 3],
    /// Linear fog colour (`fogColor`).
    pub fog_color: [f32; 3],
    /// Fog start and end distances in blocks (`fogStart`, `fogEnd`).
    pub fog_range: [f32; 2],
    /// Render distance in blocks (`far`).
    pub render_distance: f32,
    /// Night vision strength `0..1` (`nightVision`).
    pub night_vision: f32,
    /// Blindness strength `0..1` (`blindness`).
    pub blindness: f32,
    /// Darkness effect strength `0..1` (`darknessFactor`).
    pub darkness: f32,
    /// `block.properties`/`item.properties` id of the main-hand item, -1 when unmapped
    /// (`heldItemId`).
    pub held_item_id: i32,
    /// Light emitted by the main-hand item's block, `0..15` (`heldBlockLightValue`).
    pub held_block_light: i32,
    /// Off-hand counterparts (`heldItemId2`, `heldBlockLightValue2`).
    pub held_item_id2: i32,
    pub held_block_light2: i32,
    /// Player health, hunger and air as fractions `0..1`, -1 when unknown
    /// (`currentPlayerHealth`, `currentPlayerHunger`, `currentPlayerAir`).
    pub player_health: f32,
    pub player_hunger: f32,
    pub player_air: f32,
    /// Bedrock biome id at the camera entity's block, `None` before the world streams; the
    /// biome uniforms (`biome`, `biome_category`, `temperature`, `rainfall`,
    /// `biome_precipitation`) read it through its Java biome.
    pub biome: Option<u32>,
    /// Block Y the biome was sampled at, for the snow line of `biome_precipitation`.
    pub biome_block_y: i32,
    /// Sky dimension: 0 Overworld, 1 Nether, 2 End; selects the pack's `world*` folder.
    pub dimension: i32,
    /// Local player is sneaking, sprinting, hurt, burning (`is_sneaking` …).
    pub sneaking: bool,
    pub sprinting: bool,
    pub hurt: bool,
    pub on_fire: bool,
    /// Camera is on the ground (`is_on_ground`).
    pub on_ground: bool,
}

impl Default for ShaderPackFrameInputs {
    fn default() -> Self {
        Self {
            world_time: 6000,
            world_day: 0,
            moon_phase: 0,
            celestial_angle: 0.0,
            rain_strength: 0.0,
            thunder_strength: 0.0,
            eye_brightness: [0, 240],
            eye_medium: 0,
            sky_color: [0.47, 0.65, 1.0],
            fog_color: [0.75, 0.85, 1.0],
            fog_range: [0.0, 256.0],
            render_distance: 256.0,
            night_vision: 0.0,
            blindness: 0.0,
            darkness: 0.0,
            held_item_id: -1,
            held_block_light: 0,
            held_item_id2: -1,
            held_block_light2: 0,
            player_health: -1.0,
            player_hunger: -1.0,
            player_air: -1.0,
            biome: None,
            biome_block_y: 64,
            dimension: 0,
            sneaking: false,
            sprinting: false,
            hurt: false,
            on_fire: false,
            on_ground: true,
        }
    }
}

/// The effects the loaded pack draws itself on [`ShaderPackRendering`] cameras, through its
/// own programs or not at all (`clouds=off`); the Vanilla renderers leave them out of those
/// views. Written in the render world by the pack host.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShaderPackEffects {
    pub particles: bool,
    pub weather: bool,
    pub lightning: bool,
    pub clouds: bool,
    /// The block selection outline.
    pub selection: bool,
    /// Block cracks (with the selection highlight, which shares their draw).
    pub cracks: bool,
}

/// Which render-world views the pack draws an effect for, for the Vanilla renderers' queue
/// systems.
#[derive(SystemParam)]
pub struct ShaderPackDrawnEffects<'w, 's> {
    effects: Option<Res<'w, ShaderPackEffects>>,
    cameras: Query<'w, 's, (), With<ShaderPackRendering>>,
}

impl ShaderPackDrawnEffects<'_, '_> {
    /// Whether the pack draws the effect `which` picks on render-world view `view`.
    #[must_use]
    pub fn replaces(&self, view: Entity, which: impl Fn(&ShaderPackEffects) -> bool) -> bool {
        self.effects.as_deref().is_some_and(which) && self.cameras.contains(view)
    }
}
