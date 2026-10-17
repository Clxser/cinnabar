use bevy::prelude::Resource;

use super::WorldClock;

/// Validated mod output, applied only to a copy used by atmosphere rendering.
#[derive(Resource, Default)]
pub(crate) struct VisualTimeOverride(pub(crate) Option<u32>);

impl VisualTimeOverride {
    /// Keeps the server snapshot intact while fixing the rendered clock.
    pub(crate) fn rendering_clock(&self, server: WorldClock) -> WorldClock {
        match self.0 {
            Some(ticks) => WorldClock {
                server_time: Some(f64::from(ticks)),
                server_time_anchor_seconds: Some(0.0),
                daylight_cycle_enabled: false,
                ..server
            },
            None => server,
        }
    }
}

/// Local renderer-only time used by the Enhanced-mode lighting probe.
///
/// This stays separate from mod output so a local component cannot accidentally
/// claim ownership of the built-in debug control.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DebugTimeOverride {
    pub(crate) ticks: Option<u32>,
}

/// Developer-only `--fixed-time` pin for captures that compare against a reference renderer.
///
/// Set once at launch from the command line and never changed; it outranks every other
/// override so a mod's time changer or the Enhanced preset key cannot move a capture's clock.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FixedTime(pub(crate) Option<u32>);

/// The clock the renderer draws with. `--fixed-time` wins, then the Enhanced debug preset,
/// then mod output; without any of them the server clock passes through unchanged.
pub(crate) fn rendered_clock(
    server: WorldClock,
    fixed: Option<&FixedTime>,
    debug: Option<&DebugTimeOverride>,
    visual: Option<&VisualTimeOverride>,
) -> WorldClock {
    let ticks = fixed
        .and_then(|fixed| fixed.0)
        .or_else(|| debug.and_then(|debug| debug.ticks))
        .or_else(|| visual.and_then(|visual| visual.0));
    VisualTimeOverride(ticks).rendering_clock(server)
}

#[cfg(test)]
mod tests;
