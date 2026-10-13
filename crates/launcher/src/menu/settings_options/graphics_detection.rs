//! Automatic recommendations never take ownership of manually edited settings.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{SETTINGS_OPTIONS, SettingsOptions, video_presets::VideoPreset};

/// Increment when measurements or score mapping change, invalidating old results.
pub const BENCHMARK_VERSION: u32 = 2;
/// The interactive benchmark stops accepting work at this deadline.
pub const BENCHMARK_LIMIT: std::time::Duration = std::time::Duration::from_millis(2_500);
pub const TUNED_MESSAGE: &str =
    "Graphics settings tuned for this device — change in Video settings.";

/// Measurements from a single core and the terrain pipeline at the window resolution.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Scores {
    /// Nanoseconds at the 99th percentile, keyed by the measured MSAA sample count.
    pub gpu_p99_ns: BTreeMap<u32, u64>,
    pub sections_per_second: u32,
    pub duration_ms: u32,
}

/// One row supplies all admission thresholds for an existing preset.
pub struct Tier {
    pub preset: VideoPreset,
    pub gpu_budget_ns: u64,
    pub sections_per_second: u32,
    pub terrain_budget_bytes: usize,
}

/// Reserve at least 6.6 ms of the 16.7 ms frame for other render and simulation work.
/// Higher tiers reserve more time for clouds and post-processing. CPU thresholds
/// represent 1, 2 and 4 lit section rebuilds per 60 Hz frame on a single core.
/// These synthetic admission thresholds require hardware validation; they cannot
/// promise frame pacing in every server world or resource pack.
pub const TIERS: [Tier; VideoPreset::ALL.len()] = [
    Tier {
        preset: VideoPreset::Performance,
        gpu_budget_ns: 10_000_000,
        sections_per_second: 60,
        terrain_budget_bytes: assets::texture::policy::MIN_TERRAIN_BYTES,
    },
    Tier {
        preset: VideoPreset::Balanced,
        gpu_budget_ns: 7_000_000,
        sections_per_second: 120,
        terrain_budget_bytes: assets::texture::policy::MIN_TERRAIN_BYTES * 4,
    },
    Tier {
        preset: VideoPreset::Quality,
        gpu_budget_ns: 4_000_000,
        sections_per_second: 240,
        terrain_budget_bytes: assets::texture::policy::MIN_TERRAIN_BYTES * 8,
    },
];

/// Measures only sample counts consumed by the preset table, sharing their admission budgets.
pub fn sample_budgets(settings: &SettingsOptions) -> BTreeMap<u32, u64> {
    let mut budgets: BTreeMap<u32, u64> = BTreeMap::new();
    for tier in &TIERS {
        let samples = settings
            .video_preset_values(tier.preset)
            .find(|(name, _)| *name == "msaa")
            .expect("presets include MSAA")
            .1 as u32;
        budgets
            .entry(samples)
            .and_modify(|budget| *budget = (*budget).max(tier.gpu_budget_ns))
            .or_insert(tier.gpu_budget_ns);
    }
    budgets
}

/// A measured preset with distance bounded by the existing device recommendation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Recommendation {
    pub preset: VideoPreset,
    pub render_distance: u8,
}

/// Selects the highest admitted tier, then steps down once on battery power.
pub fn recommend(
    settings: &SettingsOptions,
    scores: &Scores,
    terrain_budget_bytes: usize,
    on_battery: bool,
) -> Recommendation {
    let mut selected = 0;
    let mut admitted = false;
    for (index, tier) in TIERS.iter().enumerate() {
        let samples = settings
            .video_preset_values(tier.preset)
            .find(|(name, _)| *name == "msaa")
            .expect("presets include MSAA")
            .1 as u32;
        if scores
            .gpu_p99_ns
            .get(&samples)
            .is_some_and(|&ns| ns > 0 && ns <= tier.gpu_budget_ns)
            && scores.sections_per_second >= tier.sections_per_second
            && terrain_budget_bytes >= tier.terrain_budget_bytes
        {
            selected = index;
            admitted = true;
        }
    }
    if on_battery {
        selected = selected.saturating_sub(1);
    }
    let preset = TIERS[selected].preset;
    let defaults = settings.render_distance_defaults();
    let distance = settings
        .video_preset_values(preset)
        .find(|(name, _)| *name == "render_distance")
        .expect("presets include distance")
        .1 as u8;
    Recommendation {
        preset,
        render_distance: if admitted {
            distance
        } else {
            ui::MIN_RENDER_DISTANCE_CHUNKS
        }
        .min(defaults.recommended)
        .min(defaults.native_maximum),
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct CachedScores {
    key: String,
    scores: Scores,
}

/// Persisted defaults, protected choices and a bounded cache of device measurements.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    automatic: BTreeMap<String, i32>,
    protected: BTreeSet<String>,
    cache: Vec<CachedScores>,
}

impl Preferences {
    /// Reads automatic defaults only while the user has not claimed the option.
    pub(super) fn value(&self, name: &str) -> Option<i32> {
        (!self.protected.contains(name))
            .then(|| self.automatic.get(name).copied())
            .flatten()
    }

    /// Remembers explicit edits even when their value equals the ordinary default.
    pub(super) fn protect(&mut self, name: &str) -> bool {
        VideoPreset::controls(name) && self.protected.insert(name.to_owned())
    }

    /// Rejects unknown options, invalid measurements and oversized cache records.
    pub(super) fn validated(mut self) -> Self {
        self.automatic.retain(|name, value| {
            VideoPreset::controls(name)
                && SETTINGS_OPTIONS.iter().any(|option| {
                    option.name == name
                        && (option.min..=option.max).contains(value)
                        && (*value - option.min) % option.step == 0
                })
        });
        self.protected.retain(|name| VideoPreset::controls(name));
        self.cache
            .retain(|entry| entry.key.len() == 64 && entry.scores.valid());
        self.cache.truncate(4);
        self
    }
}

impl Scores {
    /// Accepts only completed, finite-size runs with supported timestamp samples.
    fn valid(&self) -> bool {
        self.sections_per_second > 0
            && self.duration_ms > 0
            && self.duration_ms <= BENCHMARK_LIMIT.as_millis() as u32 + 500
            && !self.gpu_p99_ns.is_empty()
            && self
                .gpu_p99_ns
                .iter()
                .all(|(samples, ns)| ui::ANTI_ALIASING_SAMPLE_COUNTS.contains(samples) && *ns > 0)
    }
}

impl SettingsOptions {
    /// Returns scores only for exactly the same adapter, driver and workload identity.
    pub fn graphics_scores(&self, key: &str) -> Option<&Scores> {
        self.graphics_detection
            .cache
            .iter()
            .find(|entry| entry.key == key)
            .map(|entry| &entry.scores)
    }

    /// Saves completed measurements, keeping at most four adapter identities.
    pub fn cache_graphics_scores(&mut self, key: String, scores: Scores) -> bool {
        if key.len() != 64 || !scores.valid() {
            return false;
        }
        self.graphics_detection
            .cache
            .retain(|entry| entry.key != key);
        self.graphics_detection
            .cache
            .insert(0, CachedScores { key, scores });
        self.graphics_detection.cache.truncate(4);
        true
    }

    /// Refreshes untouched settings through the existing preset table; user values win.
    pub fn apply_graphics_recommendation(&mut self, recommendation: Recommendation) -> bool {
        let automatic = self
            .video_preset_values(recommendation.preset)
            .map(|(name, value)| {
                (
                    name.to_owned(),
                    if name == "render_distance" {
                        i32::from(recommendation.render_distance)
                    } else {
                        value
                    },
                )
            })
            .collect();
        if self.graphics_detection.automatic == automatic {
            return false;
        }
        self.graphics_detection.automatic = automatic;
        true
    }
}

#[cfg(test)]
mod tests;
