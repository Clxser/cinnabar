//! Java Edition's sky and fog colours, as Iris hands them to packs: `skyColor` is the
//! level's sky colour at the camera entity and `fogColor` the colour the fog renderer
//! computes for the frame (which Iris also clears `colortex0` to).
//!
//! Behaviour follows Java Edition as the Minecraft Wiki describes it:
//! - Every biome has a sky colour (overworld biomes derive it from their temperature), a fog
//!   colour (`#C0D8FF` in the overworld) and a water fog colour. The level blends each over
//!   the biomes around the camera with a Gaussian kernel; the caller supplies that blend as
//!   weighted biomes ([`BiomeColors::blend`]).
//! - The sky colour dims with the time of day and greys out in rain and thunder.
//! - The fog colour dims with the time of day, takes the sunrise/sunset glow when the camera
//!   faces the sun's side of the sky, mixes towards the sky colour more the shorter the
//!   render distance, and darkens in rain, in thunder and below the world's floor.
//!
//! Not modelled: lightning flashes (no bolt source), the boss-fight darkening, blindness and
//! darkness fog, and the underwater colour easing between biomes and the water-vision ramp
//! (the biome's water fog colour is used directly).

use assets::facts::java_mapping::{JavaBiomeEntry, JavaBlockMapping};

/// What Java reads unmapped biomes as (see [`crate::BiomeUniforms`]).
const FALLBACK_BIOME: &str = "minecraft:the_void";
/// The overworld's fog colour, shared by every overworld biome but the pale garden.
const OVERWORLD_FOG: u32 = 0xC0_D8_FF;
/// The default water fog colour.
const DEFAULT_WATER_FOG: u32 = 0x05_05_33;
/// `(name, sky, fog, water fog)` overrides of one biome.
type BiomeColourOverride = (&'static str, Option<u32>, Option<u32>, Option<u32>);
/// Biomes whose colours are not the defaults; `None` keeps the default (sky from the
/// temperature, [`OVERWORLD_FOG`], [`DEFAULT_WATER_FOG`]).
const BIOME_COLOURS: &[BiomeColourOverride] = &[
    (
        "minecraft:pale_garden",
        Some(0xB9_B9_B9),
        Some(0x81_77_70),
        Some(0x55_69_80),
    ),
    ("minecraft:swamp", None, None, Some(0x23_23_17)),
    ("minecraft:mangrove_swamp", None, None, Some(0x4D_7A_60)),
    ("minecraft:cherry_grove", None, None, Some(0x5D_B7_EF)),
    ("minecraft:warm_ocean", None, None, Some(0x04_1F_33)),
    ("minecraft:lukewarm_ocean", None, None, Some(0x04_16_33)),
    (
        "minecraft:deep_lukewarm_ocean",
        None,
        None,
        Some(0x04_16_33),
    ),
    ("minecraft:nether_wastes", None, Some(0x33_08_08), None),
    ("minecraft:soul_sand_valley", None, Some(0x1B_47_45), None),
    ("minecraft:crimson_forest", None, Some(0x33_03_03), None),
    ("minecraft:warped_forest", None, Some(0x1A_05_1A), None),
    ("minecraft:basalt_deltas", None, Some(0x68_5F_70), None),
    ("minecraft:the_end", Some(0), Some(0xA0_80_A0), None),
    ("minecraft:end_highlands", Some(0), Some(0xA0_80_A0), None),
    ("minecraft:end_midlands", Some(0), Some(0xA0_80_A0), None),
    (
        "minecraft:small_end_islands",
        Some(0),
        Some(0xA0_80_A0),
        None,
    ),
    ("minecraft:end_barrens", Some(0), Some(0xA0_80_A0), None),
];
/// Fog colours in lava and in powder snow.
const LAVA_FOG: [f32; 3] = [0.6, 0.1, 0.0];
const POWDER_SNOW_FOG: [f32; 3] = [0.623, 0.734, 0.785];

/// An 8-bit `0xRRGGBB` colour as floats.
fn rgb(colour: u32) -> [f32; 3] {
    [16, 8, 0].map(|shift| ((colour >> shift) & 0xFF) as f32 / 255.0)
}

/// A float colour through an 8-bit channel each, as Java packs colours into ints.
fn quantize(colour: [f32; 3]) -> [f32; 3] {
    colour.map(|c| (c * 255.0).floor().clamp(0.0, 255.0) / 255.0)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

/// Java's sky colour for a biome temperature: a hue and saturation that shift with the
/// clamped temperature, at full value, truncated to 8 bits per channel.
fn temperature_sky(temperature: f32) -> [f32; 3] {
    let t = (temperature / 3.0).clamp(-1.0, 1.0);
    let (hue, saturation) = (0.622_222_24 - t * 0.05, 0.5 + t * 0.1);
    let sector = (hue * 6.0).floor();
    let f = hue * 6.0 - sector;
    let (p, q, u) = (
        1.0 - saturation,
        1.0 - f * saturation,
        1.0 - (1.0 - f) * saturation,
    );
    let colour = match sector as i32 % 6 {
        0 => [1.0, u, p],
        1 => [q, 1.0, p],
        2 => [p, 1.0, u],
        3 => [p, q, 1.0],
        4 => [u, p, 1.0],
        _ => [1.0, p, q],
    };
    quantize(colour)
}

/// A Java biome's sky, fog and water fog colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BiomeColors {
    pub sky: [f32; 3],
    pub fog: [f32; 3],
    pub water_fog: [f32; 3],
}

impl BiomeColors {
    /// Every colour 0, for blending.
    const ZERO: BiomeColors = BiomeColors {
        sky: [0.0; 3],
        fog: [0.0; 3],
        water_fog: [0.0; 3],
    };

    /// The colours of `entry`.
    pub fn of(entry: &JavaBiomeEntry<'_>) -> Self {
        let (sky, fog, water_fog) = BIOME_COLOURS
            .iter()
            .find(|(name, ..)| *name == entry.name)
            .map_or((None, None, None), |(_, sky, fog, water)| {
                (*sky, *fog, *water)
            });
        Self {
            sky: sky.map_or_else(|| temperature_sky(entry.temperature), rgb),
            fog: rgb(fog.unwrap_or(OVERWORLD_FOG)),
            water_fog: rgb(water_fog.unwrap_or(DEFAULT_WATER_FOG)),
        }
    }

    /// The colours of the Java biome of Bedrock biome `bedrock_biome`; unmapped biomes read
    /// as the void, as for the biome uniforms. `None` without a Java biome registry.
    pub fn at(mapping: &JavaBlockMapping, bedrock_biome: u32) -> Option<Self> {
        let index = mapping.java_biome_index(bedrock_biome).or_else(|| {
            mapping
                .java_biomes()
                .position(|biome| biome.name == FALLBACK_BIOME)
                .and_then(|index| u16::try_from(index).ok())
        })?;
        mapping
            .java_biome_entry(index)
            .map(|entry| Self::of(&entry))
    }

    /// The colours of the void: what a camera without streamed biomes sees.
    pub fn fallback(mapping: &JavaBlockMapping) -> Option<Self> {
        mapping
            .java_biomes()
            .find(|biome| biome.name == FALLBACK_BIOME)
            .map(|entry| Self::of(&entry))
    }

    /// The weighted average of `samples`; `None` when no sample has weight.
    pub fn blend(samples: impl IntoIterator<Item = (BiomeColors, f32)>) -> Option<Self> {
        let (sum, total) = samples.into_iter().filter(|(_, w)| *w > 0.0).fold(
            (Self::ZERO, 0.0f32),
            |(sum, total), (colours, w)| {
                let add = |a: [f32; 3], b: [f32; 3]| std::array::from_fn(|i| a[i] + b[i] * w);
                (
                    Self {
                        sky: add(sum.sky, colours.sky),
                        fog: add(sum.fog, colours.fog),
                        water_fog: add(sum.water_fog, colours.water_fog),
                    },
                    total + w,
                )
            },
        );
        (total > 0.0).then(|| {
            let scale = |c: [f32; 3]| c.map(|v| v / total);
            Self {
                sky: scale(sum.sky),
                fog: scale(sum.fog),
                water_fog: scale(sum.water_fog),
            }
        })
    }
}

/// The dimension types whose sky and fog behave differently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkyDimension {
    Overworld,
    Nether,
    End,
}

impl SkyDimension {
    /// The lowest block of the dimension, below which the fog fades to black.
    fn floor(self) -> f32 {
        match self {
            Self::Overworld => -64.0,
            Self::Nether | Self::End => 0.0,
        }
    }
}

/// The medium the camera is in, which replaces the fog colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FogMedium {
    Air,
    Water,
    Lava,
    PowderSnow,
}

/// The state the sky and fog colours depend on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyFogInputs {
    /// The biome colours blended around the camera.
    pub biome: BiomeColors,
    pub dimension: SkyDimension,
    /// Fraction of a turn with 0 at noon (Java's time of day).
    pub celestial_angle: f32,
    pub rain: f32,
    pub thunder: f32,
    /// Render distance in chunks.
    pub render_distance_chunks: f32,
    /// The camera's world-space view direction, unit length.
    pub look: [f32; 3],
    /// The camera's world Y.
    pub camera_y: f32,
    pub medium: FogMedium,
    /// Night vision strength `0..1`.
    pub night_vision: f32,
}

/// `skyColor` and `fogColor`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyFogColors {
    pub sky: [f32; 3],
    pub fog: [f32; 3],
}

/// How bright the sky is at `celestial_angle`: full from mid-morning to mid-afternoon, dark
/// through the night.
fn daylight(celestial_angle: f32) -> f32 {
    ((celestial_angle * std::f32::consts::TAU).cos() * 2.0 + 0.5).clamp(0.0, 1.0)
}

/// The sunrise/sunset glow at `celestial_angle`, RGB plus its strength; `None` while the sun
/// is well above or below the horizon.
pub fn sunrise_color(celestial_angle: f32) -> Option<[f32; 4]> {
    let height = (celestial_angle * std::f32::consts::TAU).cos();
    if !(-0.4..=0.4).contains(&height) {
        return None;
    }
    let g = height / 0.4 * 0.5 + 0.5;
    let strength = 1.0 - (1.0 - (g * std::f32::consts::PI).sin()) * 0.99;
    Some([g * 0.3 + 0.7, g * g * 0.7 + 0.2, 0.2, strength * strength])
}

/// The level's sky colour: the biome sky dimmed by the time of day, then greyed by rain and
/// by thunder. Only the overworld has a time of day; the nether's is fixed at midnight and
/// the end's biomes have a black sky, so both are black.
fn sky_color(inputs: &SkyFogInputs) -> [f32; 3] {
    if inputs.dimension != SkyDimension::Overworld {
        return [0.0; 3];
    }
    let mut colour = inputs
        .biome
        .sky
        .map(|c| c * daylight(inputs.celestial_angle));
    let luminance = |c: [f32; 3]| c[0] * 0.3 + c[1] * 0.59 + c[2] * 0.11;
    for (level, grey) in [(inputs.rain, 0.6), (inputs.thunder, 0.2)] {
        if level > 0.0 {
            let grey = luminance(colour) * grey;
            colour = mix(colour, [grey; 3], level * 0.75);
        }
    }
    quantize(colour)
}

/// The fog colour in air: the biome fog dimmed by the time of day, the sunrise glow towards
/// the sun's side, then mixed towards the sky by the render distance.
fn air_fog(inputs: &SkyFogInputs, sky: [f32; 3]) -> [f32; 3] {
    let light = daylight(inputs.celestial_angle);
    let mut fog = match inputs.dimension {
        SkyDimension::Overworld => {
            let [r, g, b] = inputs.biome.fog;
            [
                r * (light * 0.94 + 0.06),
                g * (light * 0.94 + 0.06),
                b * (light * 0.91 + 0.09),
            ]
        }
        SkyDimension::Nether => inputs.biome.fog,
        SkyDimension::End => inputs.biome.fog.map(|c| c * 0.15),
    };
    let chunks = inputs.render_distance_chunks;
    if inputs.dimension == SkyDimension::Overworld && chunks >= 4.0 {
        let sun_side = if (inputs.celestial_angle * std::f32::consts::TAU).sin() > 0.0 {
            -1.0
        } else {
            1.0
        };
        let facing = (inputs.look[0] * sun_side).max(0.0);
        if let Some([r, g, b, strength]) = sunrise_color(inputs.celestial_angle)
            && facing > 0.0
        {
            fog = mix(fog, [r, g, b], facing * strength);
        }
    }
    let distance = 1.0 - (0.25 + 0.75 * chunks.min(32.0) / 32.0).powf(0.25);
    mix(fog, sky, distance)
}

/// `skyColor` and `fogColor` for one frame and view.
pub fn sky_fog(inputs: &SkyFogInputs) -> SkyFogColors {
    let sky = sky_color(inputs);
    let mut fog = match inputs.medium {
        FogMedium::Air => air_fog(inputs, sky),
        FogMedium::Water => inputs.biome.water_fog,
        FogMedium::Lava => LAVA_FOG,
        FogMedium::PowderSnow => POWDER_SNOW_FOG,
    };
    if inputs.medium == FogMedium::Air {
        let [r, g, b] = fog;
        let rain = inputs.rain;
        fog = [
            r * (1.0 - rain * 0.5),
            g * (1.0 - rain * 0.5),
            b * (1.0 - rain * 0.4),
        ];
        fog = fog.map(|c| c * (1.0 - inputs.thunder * 0.5));
    }
    let above_floor = inputs.camera_y - inputs.dimension.floor();
    if above_floor < 1.0 && matches!(inputs.medium, FogMedium::Air | FogMedium::Water) {
        let fade = above_floor.max(0.0);
        fog = fog.map(|c| c * fade * fade);
    }
    let vision = inputs.night_vision;
    if vision > 0.0 && inputs.medium != FogMedium::Water {
        let brightest = fog.into_iter().fold(0.0f32, f32::max);
        if brightest > 0.0 {
            fog = fog.map(|c| c * (1.0 - vision) + c / brightest * vision);
        }
    }
    SkyFogColors { sky, fog }
}

#[cfg(test)]
mod tests;
