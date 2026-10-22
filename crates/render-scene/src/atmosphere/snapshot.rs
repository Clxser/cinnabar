//! Conversion between prepared native atmosphere and engine-independent scene snapshots.

use super::*;

const _: () = assert!(
    std::mem::size_of::<AtmosphereFrame>()
        == std::mem::size_of::<render_model::environment_snapshot::AtmosphereSnapshot>()
);

impl AtmosphereFrame {
    /// Captures every prepared shader channel with no dependency on a recording or live session.
    pub fn snapshot(self) -> render_model::environment_snapshot::AtmosphereSnapshot {
        let Self {
            sun_direction_daylight,
            moon_direction_phase,
            sky_zenith_rain,
            sky_horizon_thunder,
            fog_color_start,
            fog_end_time,
            sunrise_band,
            sky_extra,
            liquid_distance,
        } = self;
        render_model::environment_snapshot::AtmosphereSnapshot {
            sun_direction_daylight: sun_direction_daylight.to_array(),
            moon_direction_phase: moon_direction_phase.to_array(),
            sky_zenith_rain: sky_zenith_rain.to_array(),
            sky_horizon_thunder: sky_horizon_thunder.to_array(),
            fog_color_start: fog_color_start.to_array(),
            fog_end_time: fog_end_time.to_array(),
            sunrise_band: sunrise_band.to_array(),
            sky_extra: sky_extra.to_array(),
            liquid_distance: liquid_distance.to_array(),
        }
    }

    /// Admits an independent replay frame; source light values are not recalculated from today's clock.
    pub fn from_snapshot(
        snapshot: render_model::environment_snapshot::AtmosphereSnapshot,
    ) -> Option<Self> {
        use render_model::environment_snapshot::AtmosphereSnapshot;
        let AtmosphereSnapshot {
            sun_direction_daylight,
            moon_direction_phase,
            sky_zenith_rain,
            sky_horizon_thunder,
            fog_color_start,
            fog_end_time,
            sunrise_band,
            sky_extra,
            liquid_distance,
        } = AtmosphereSnapshot::from_words(snapshot.words())?;
        Some(Self {
            sun_direction_daylight: Vec4::from_array(sun_direction_daylight),
            moon_direction_phase: Vec4::from_array(moon_direction_phase),
            sky_zenith_rain: Vec4::from_array(sky_zenith_rain),
            sky_horizon_thunder: Vec4::from_array(sky_horizon_thunder),
            fog_color_start: Vec4::from_array(fog_color_start),
            fog_end_time: Vec4::from_array(fog_end_time),
            sunrise_band: Vec4::from_array(sunrise_band),
            sky_extra: Vec4::from_array(sky_extra),
            liquid_distance: Vec4::from_array(liquid_distance),
        })
    }
}
