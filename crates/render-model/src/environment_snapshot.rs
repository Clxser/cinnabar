//! Engine-independent prepared environment values used by native scene recordings.

use bytemuck::{Pod, Zeroable};

use crate::LightmapInputs;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct AtmosphereSnapshot {
    pub sun_direction_daylight: [f32; 4],
    pub moon_direction_phase: [f32; 4],
    pub sky_zenith_rain: [f32; 4],
    pub sky_horizon_thunder: [f32; 4],
    pub fog_color_start: [f32; 4],
    pub fog_end_time: [f32; 4],
    pub sunrise_band: [f32; 4],
    pub sky_extra: [f32; 4],
    pub liquid_distance: [f32; 4],
}

pub const ATMOSPHERE_SNAPSHOT_WORDS: usize =
    std::mem::size_of::<AtmosphereSnapshot>() / std::mem::size_of::<u32>();

impl AtmosphereSnapshot {
    /// Word encoding preserves finite float bits while callers choose the on-disk byte order.
    pub fn words(self) -> [u32; ATMOSPHERE_SNAPSHOT_WORDS] {
        bytemuck::cast(self)
    }

    /// Untrusted scene data cannot inject non-finite inputs into atmosphere passes.
    pub fn from_words(words: [u32; ATMOSPHERE_SNAPSHOT_WORDS]) -> Option<Self> {
        let value: Self = bytemuck::cast(words);
        words
            .iter()
            .all(|word| f32::from_bits(*word).is_finite())
            .then_some(value)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LightmapSnapshot {
    ramp: [f32; 16],
    sky_darken: f32,
    sunrise: [f32; 4],
    brightness: f32,
    night_vision: f32,
    darkness: f32,
    darkness_pulse: f32,
    flags: u32,
    dimension_tint: [f32; 3],
}

pub const LIGHTMAP_SNAPSHOT_WORDS: usize =
    std::mem::size_of::<LightmapSnapshot>() / std::mem::size_of::<u32>();
const LIGHTNING: u32 = 1;
const AMBIENT_ADJUSTMENT: u32 = 2;
const DIMENSION_TINT: u32 = 4;

impl LightmapSnapshot {
    /// Exhaustive source binding makes new lightmap channels require an explicit recording decision.
    pub fn capture(inputs: LightmapInputs) -> Self {
        let LightmapInputs {
            ramp,
            sky_darken,
            sunrise,
            lightning,
            brightness,
            night_vision,
            darkness,
            darkness_pulse,
            ambient_adjustment,
            dimension_tint,
        } = inputs;
        Self {
            ramp,
            sky_darken,
            sunrise,
            brightness,
            night_vision,
            darkness,
            darkness_pulse,
            flags: u32::from(lightning) * LIGHTNING
                | u32::from(ambient_adjustment) * AMBIENT_ADJUSTMENT
                | u32::from(dimension_tint.is_some()) * DIMENSION_TINT,
            dimension_tint: dimension_tint.unwrap_or([0.0; 3]),
        }
    }

    pub fn words(self) -> [u32; LIGHTMAP_SNAPSHOT_WORDS] {
        bytemuck::cast(self)
    }

    /// Reserved flags and non-finite colour values are rejected before constructing native inputs.
    pub fn from_words(words: [u32; LIGHTMAP_SNAPSHOT_WORDS]) -> Option<Self> {
        let value: Self = bytemuck::cast(words);
        if value.flags & !(LIGHTNING | AMBIENT_ADJUSTMENT | DIMENSION_TINT) != 0 {
            return None;
        }
        let floats = value
            .ramp
            .iter()
            .chain(std::iter::once(&value.sky_darken))
            .chain(value.sunrise.iter())
            .chain([
                &value.brightness,
                &value.night_vision,
                &value.darkness,
                &value.darkness_pulse,
            ])
            .chain(value.dimension_tint.iter());
        floats
            .into_iter()
            .all(|value| value.is_finite())
            .then_some(value)
    }

    /// Restores the exact original lightmap inputs independently of the current player's settings.
    pub fn restore(self) -> LightmapInputs {
        let Self {
            ramp,
            sky_darken,
            sunrise,
            brightness,
            night_vision,
            darkness,
            darkness_pulse,
            flags,
            dimension_tint,
        } = self;
        LightmapInputs {
            ramp,
            sky_darken,
            sunrise,
            brightness,
            night_vision,
            darkness,
            darkness_pulse,
            lightning: flags & LIGHTNING != 0,
            ambient_adjustment: flags & AMBIENT_ADJUSTMENT != 0,
            dimension_tint: (flags & DIMENSION_TINT != 0).then_some(dimension_tint),
        }
    }
}
