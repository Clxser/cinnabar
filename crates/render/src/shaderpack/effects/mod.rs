//! What the pack draws besides terrain and the sky discs, through its own programs as Iris
//! does: the sun and moon (`gbuffers_skytextured`), stars (`gbuffers_skybasic`), lightning
//! (`gbuffers_lightning`), block cracks (`gbuffers_damagedblock`), the block selection
//! (`gbuffers_line`), particles (`gbuffers_particles`, `gbuffers_particles_translucent`),
//! clouds (`gbuffers_clouds`) and rain and snow (`gbuffers_weather`).
//!
//! # Geometry
//! Every effect is rebuilt on the CPU each frame from the data Cinnabar's own renderers draw
//! (particle instances, precipitation layers, bolt ribbons, crack and selection vertices,
//! the cloud mesher) into the 52-byte Iris vertex, positioned relative to the camera as
//! Java's renderers do, so the draws use `gbufferModelView` with a zero `chunkOffset`. Quads
//! are indexed; [`EffectBuffers`] holds one view's vertices and indices.
//!
//! The sun, moon and stars follow Java's `SkyRenderer`: a 30-block sun and 20-block moon
//! quad 100 blocks out and Java's 1500-candidate star field (seed 10842, `java.util.Random`),
//! turned by `rotY(-90°)·rotZ(sunPathRotation)·rotX(angle·360°)`, coloured
//! `(1, 1, 1, 1 - rain)` and `starBrightness` as Java's colour modulator.
//!
//! # Frame order
//! As Iris records them: sun, moon and stars after the sky discs; lightning, cracks and the
//! selection after opaque terrain, before the deferred passes; particles by the pack's
//! `particles.ordering` (opaque before deferred unless `AFTER`, translucent after translucent
//! terrain unless `BEFORE`); then clouds and weather.
//!
//! # Pack settings
//! `clouds=off|fast|fancy` (fast: flat cloud caps), `sun`, `moon`, `stars`,
//! `weather=<bool>`, `rain.depth` and the particle ordering directives
//! (`particles.ordering`, `particles.before.deferred`, `separateEntityDraws`).
//!
//! # Textures
//! `gtexture` binds the effect's own texture (see [`textures`]); `minecraft:` resource
//! references in `shaders.properties` resolve through [`resources`].

mod celestial;
mod clouds;
mod frame;
mod lines;
mod particles;
pub(crate) mod resources;
mod stars;
pub(crate) mod textures;
mod vertices;
mod weather;

#[cfg(test)]
mod tests;

pub(crate) use frame::{EffectBuffers, EffectCamera, EffectInputs, prepare_effects};

use super::alpha_test::ONE_TENTH_ALPHA;
use super::draws::TRANSLUCENT_BLEND;
use shaderpack::{AlphaTest, BlendFactor, BlendMode, ShaderProperties};

/// Iris's `renderStage` values (`MC_RENDER_STAGE_*`) of the effects.
mod render_stage {
    pub(super) const SUN: i32 = 4;
    pub(super) const MOON: i32 = 5;
    pub(super) const STARS: i32 = 6;
    pub(super) const ENTITIES: i32 = 11;
    pub(super) const DESTROY: i32 = 13;
    pub(super) const OUTLINE: i32 = 14;
    pub(super) const PARTICLES: i32 = 19;
    pub(super) const CLOUDS: i32 = 20;
    pub(super) const RAIN_SNOW: i32 = 21;
}

/// One effect draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Effect {
    Sun,
    Moon,
    Stars,
    /// Bolt ribbons, drawn in the entity phase.
    Lightning,
    /// Block cracks.
    DamagedBlock,
    /// The block selection outline.
    Outline,
    /// Opaque and alpha-tested particles.
    Particles,
    /// Alpha-blended particles.
    ParticlesTranslucent,
    /// Bedrock's additive particles, through the translucent particle program.
    ParticlesAdditive,
    Clouds,
    /// Rain and snow.
    Weather,
}

/// Java's `BlendFunction.OVERLAY` of the sun, moon and stars: `SRC_ALPHA ONE ONE ZERO`.
const CELESTIAL_BLEND: BlendMode = BlendMode::Factors {
    src_color: BlendFactor::SrcAlpha,
    dst_color: BlendFactor::One,
    src_alpha: BlendFactor::One,
    dst_alpha: BlendFactor::Zero,
};
/// Java's lightning blending: `SRC_ALPHA ONE`.
const ADDITIVE_BLEND: BlendMode = BlendMode::Factors {
    src_color: BlendFactor::SrcAlpha,
    dst_color: BlendFactor::One,
    src_alpha: BlendFactor::SrcAlpha,
    dst_alpha: BlendFactor::One,
};
/// Java's crack blending: `DST_COLOR SRC_COLOR ONE ZERO`.
const CRUMBLING_BLEND: BlendMode = BlendMode::Factors {
    src_color: BlendFactor::DstColor,
    dst_color: BlendFactor::SrcColor,
    src_alpha: BlendFactor::One,
    dst_alpha: BlendFactor::Zero,
};

impl Effect {
    pub(crate) const ALL: [Effect; 11] = [
        Effect::Sun,
        Effect::Moon,
        Effect::Stars,
        Effect::Lightning,
        Effect::DamagedBlock,
        Effect::Outline,
        Effect::Particles,
        Effect::ParticlesTranslucent,
        Effect::ParticlesAdditive,
        Effect::Clouds,
        Effect::Weather,
    ];

    /// The Iris program this effect asks for first (`IrisPipelines`' shader keys).
    pub(crate) fn program(self) -> &'static str {
        match self {
            Effect::Sun | Effect::Moon => "gbuffers_skytextured",
            Effect::Stars => "gbuffers_skybasic",
            Effect::Lightning => "gbuffers_lightning",
            Effect::DamagedBlock => "gbuffers_damagedblock",
            Effect::Outline => "gbuffers_line",
            Effect::Particles => "gbuffers_particles",
            Effect::ParticlesTranslucent | Effect::ParticlesAdditive => {
                "gbuffers_particles_translucent"
            }
            Effect::Clouds => "gbuffers_clouds",
            Effect::Weather => "gbuffers_weather",
        }
    }

    pub(crate) fn render_stage(self) -> i32 {
        match self {
            Effect::Sun => render_stage::SUN,
            Effect::Moon => render_stage::MOON,
            Effect::Stars => render_stage::STARS,
            Effect::Lightning => render_stage::ENTITIES,
            Effect::DamagedBlock => render_stage::DESTROY,
            Effect::Outline => render_stage::OUTLINE,
            Effect::Particles | Effect::ParticlesTranslucent | Effect::ParticlesAdditive => {
                render_stage::PARTICLES
            }
            Effect::Clouds => render_stage::CLOUDS,
            Effect::Weather => render_stage::RAIN_SNOW,
        }
    }

    /// Drawn with the sky, behind everything and without depth.
    pub(crate) fn sky(self) -> bool {
        matches!(self, Effect::Sun | Effect::Moon | Effect::Stars)
    }

    /// Writes depth (tested whenever it is not a sky effect). Weather writes it only under
    /// `rain.depth`, as Iris picks `WEATHER_DEPTH_WRITE`.
    pub(crate) fn depth_write(self, rain_depth: bool) -> bool {
        match self {
            Effect::Sun
            | Effect::Moon
            | Effect::Stars
            | Effect::DamagedBlock
            | Effect::ParticlesAdditive => false,
            Effect::Weather => rain_depth,
            _ => true,
        }
    }

    /// Culls back faces: only clouds, as Java's cloud pipeline does.
    pub(crate) fn culls(self) -> bool {
        self == Effect::Clouds
    }

    /// Iris's alpha test of the shader key without an `alphaTest.<program>` rule.
    pub(crate) fn default_alpha_test(self) -> Option<AlphaTest> {
        matches!(
            self,
            Effect::DamagedBlock
                | Effect::Particles
                | Effect::ParticlesTranslucent
                | Effect::ParticlesAdditive
                | Effect::Clouds
                | Effect::Weather
        )
        .then_some(ONE_TENTH_ALPHA)
    }

    /// Java's blending without a `blend.<program>` rule.
    pub(crate) fn default_blend(self) -> BlendMode {
        match self {
            Effect::Sun | Effect::Moon | Effect::Stars => CELESTIAL_BLEND,
            Effect::Lightning | Effect::ParticlesAdditive => ADDITIVE_BLEND,
            Effect::DamagedBlock => CRUMBLING_BLEND,
            Effect::Particles => BlendMode::Off,
            Effect::Outline | Effect::ParticlesTranslucent | Effect::Clouds | Effect::Weather => {
                TRANSLUCENT_BLEND
            }
        }
    }
}

/// `clouds=` in `shaders.properties`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CloudSetting {
    /// The player's cloud setting.
    #[default]
    Default,
    Off,
    /// Flat cloud layers.
    Fast,
    /// Cuboid clouds.
    Fancy,
}

/// When particles draw relative to the deferred passes (Iris's `ParticleRenderingSettings`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ParticleOrdering {
    /// All before the deferred passes.
    Before,
    /// Opaque before, translucent after.
    #[default]
    Mixed,
    /// All after translucent terrain.
    After,
}

/// The pack's effect directives and ids, read once at load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EffectSettings {
    pub clouds: CloudSetting,
    pub sun: bool,
    pub moon: bool,
    pub stars: bool,
    pub weather: bool,
    /// `rain.depth`: weather writes depth.
    pub rain_depth: bool,
    pub particles: ParticleOrdering,
    /// `entityId` of `minecraft:lightning_bolt` from `entity.properties`, else -1.
    pub lightning_entity: i32,
}

impl Default for EffectSettings {
    fn default() -> Self {
        Self {
            clouds: CloudSetting::Default,
            sun: true,
            moon: true,
            stars: true,
            weather: true,
            rain_depth: false,
            particles: ParticleOrdering::Mixed,
            lightning_entity: -1,
        }
    }
}

impl EffectSettings {
    /// Reads the directives as Iris's `ShaderProperties` does. Without an ordering directive
    /// particles draw after the deferred passes when the pack has any (and no
    /// `separateEntityDraws`), else mixed.
    pub(crate) fn read(
        properties: &ShaderProperties,
        has_deferred: bool,
        lightning_entity: Option<i32>,
    ) -> Self {
        let flag = |key: &str, default: bool| match properties.get(key).map(str::trim) {
            Some("true") => true,
            Some("false") => false,
            _ => default,
        };
        let clouds = match properties.get("clouds").map(str::trim) {
            Some("off") => CloudSetting::Off,
            Some("fast") => CloudSetting::Fast,
            Some("fancy") => CloudSetting::Fancy,
            _ => CloudSetting::Default,
        };
        let weather = properties
            .get("weather")
            .and_then(|v| v.split_whitespace().next())
            .is_none_or(|first| first == "true");
        let separate = properties.get("separateEntityDraws").is_some();
        let ordering = properties
            .entries
            .iter()
            .rev()
            .find(|(key, _)| key.starts_with("particles.ordering"))
            .and_then(
                |(_, value)| match value.trim().to_ascii_uppercase().as_str() {
                    "BEFORE" => Some(ParticleOrdering::Before),
                    "MIXED" => Some(ParticleOrdering::Mixed),
                    "AFTER" => Some(ParticleOrdering::After),
                    _ => None,
                },
            );
        let particles = ordering
            .or_else(|| separate.then_some(ParticleOrdering::Mixed))
            .or_else(|| {
                flag("particles.before.deferred", false).then_some(ParticleOrdering::Before)
            })
            .unwrap_or(if has_deferred && !separate {
                ParticleOrdering::After
            } else {
                ParticleOrdering::Mixed
            });
        Self {
            clouds,
            sun: flag("sun", true),
            moon: flag("moon", true),
            stars: flag("stars", true),
            weather,
            rain_depth: flag("rain.depth", false),
            particles,
            lightning_entity: lightning_entity.unwrap_or(-1),
        }
    }

    /// Whether the pack draws `effect` at all.
    pub(crate) fn draws(&self, effect: Effect) -> bool {
        match effect {
            Effect::Sun => self.sun,
            Effect::Moon => self.moon,
            Effect::Stars => self.stars,
            Effect::Clouds => self.clouds != CloudSetting::Off,
            Effect::Weather => self.weather,
            _ => true,
        }
    }

    /// Whether `effect` draws after the deferred passes, with translucent terrain.
    pub(crate) fn after_deferred(&self, effect: Effect) -> bool {
        match effect {
            Effect::Particles => self.particles == ParticleOrdering::After,
            Effect::ParticlesTranslucent | Effect::ParticlesAdditive => {
                self.particles != ParticleOrdering::Before
            }
            Effect::Clouds | Effect::Weather => true,
            _ => false,
        }
    }

    /// `entityId` during `effect`'s draw: the lightning bolt's id, else Iris's -1.
    pub(crate) fn entity_id(&self, effect: Effect) -> i32 {
        if effect == Effect::Lightning {
            self.lightning_entity
        } else {
            -1
        }
    }
}
