use super::*;

/// Supplies a device whose distance ceiling is independent of the scores.
fn settings() -> SettingsOptions {
    let mut settings = SettingsOptions::default();
    settings.set_render_distance_device(ui::RenderDistanceDevice {
        physical_memory_bytes: 16 << 30,
        dedicated_graphics_memory_bytes: 4 << 30,
        use_full_graphics_memory: false,
    });
    settings
}

/// Creates completed measurements at a table row's exact admission thresholds.
fn scores(tier: &Tier) -> Scores {
    Scores {
        gpu_p99_ns: ui::ANTI_ALIASING_SAMPLE_COUNTS
            .into_iter()
            .map(|samples| (samples, tier.gpu_budget_ns))
            .collect(),
        sections_per_second: tier.sections_per_second,
        duration_ms: 800,
    }
}

#[test]
fn each_table_row_admits_its_existing_preset_at_the_boundary() {
    let settings = settings();
    for (index, tier) in TIERS.iter().enumerate() {
        assert_eq!(
            tier.preset,
            VideoPreset::ALL[index],
            "table covers preset ordering"
        );
        let selected = recommend(&settings, &scores(tier), tier.terrain_budget_bytes, false);
        assert_eq!(selected.preset, tier.preset);
        assert!(selected.render_distance <= settings.render_distance_defaults().recommended);
        assert!(selected.render_distance <= settings.render_distance_defaults().native_maximum);
        let battery = recommend(&settings, &scores(tier), tier.terrain_budget_bytes, true);
        assert_eq!(battery.preset, TIERS[index.saturating_sub(1)].preset);
    }
}

#[test]
fn cpu_gpu_and_memory_independently_bound_quality() {
    let settings = settings();
    let tier = &TIERS[2];
    let mut measured = scores(tier);
    measured.sections_per_second -= 1;
    assert_eq!(
        recommend(&settings, &measured, tier.terrain_budget_bytes, false).preset,
        VideoPreset::Balanced
    );
    measured = scores(tier);
    measured.gpu_p99_ns.values_mut().for_each(|ns| *ns += 1);
    assert_eq!(
        recommend(&settings, &measured, tier.terrain_budget_bytes, false).preset,
        VideoPreset::Balanced
    );
    assert_eq!(
        recommend(
            &settings,
            &scores(tier),
            tier.terrain_budget_bytes - 1,
            false
        )
        .preset,
        VideoPreset::Balanced
    );
    assert_eq!(
        recommend(&settings, &Scores::default(), 0, false).preset,
        VideoPreset::Performance
    );
}

#[test]
fn every_explicit_edit_including_a_default_wins_after_a_rerun_and_reload() {
    for option in SETTINGS_OPTIONS
        .iter()
        .filter(|option| VideoPreset::controls(option.name))
    {
        let mut settings = settings();
        let index = SETTINGS_OPTIONS
            .iter()
            .position(|candidate| candidate.name == option.name)
            .unwrap();
        let chosen = settings.get(index);
        assert!(settings.set(index, chosen));
        settings.apply_graphics_recommendation(Recommendation {
            preset: VideoPreset::Quality,
            render_distance: 24,
        });
        assert_eq!(settings.value(option.name), chosen, "{}", option.name);
        let mut loaded = SettingsOptions::decode(&serde_json::to_vec(&settings).unwrap()).unwrap();
        loaded.set_render_distance_device(ui::RenderDistanceDevice {
            physical_memory_bytes: 16 << 30,
            dedicated_graphics_memory_bytes: 4 << 30,
            use_full_graphics_memory: false,
        });
        loaded.apply_graphics_recommendation(Recommendation {
            preset: VideoPreset::Performance,
            render_distance: 8,
        });
        assert_eq!(loaded.value(option.name), chosen, "{}", option.name);
    }
}

#[test]
fn automatic_defaults_round_trip_and_update_without_becoming_user_edits() {
    let mut settings = settings();
    settings.apply_graphics_recommendation(Recommendation {
        preset: VideoPreset::Quality,
        render_distance: 24,
    });
    let mut loaded = SettingsOptions::decode(&serde_json::to_vec(&settings).unwrap()).unwrap();
    assert_eq!(loaded.value("render_distance"), 24);
    loaded.apply_graphics_recommendation(Recommendation {
        preset: VideoPreset::Performance,
        render_distance: 8,
    });
    assert_eq!(loaded.value("render_distance"), 8);
    assert_eq!(loaded.value("msaa"), 1);
    assert_eq!(loaded.value("render_clouds"), 0);
}

#[test]
fn manual_preset_selection_protects_its_defaults_from_detection() {
    for preset in VideoPreset::ALL {
        let mut settings = settings();
        settings.apply_video_preset(preset);
        let expected = settings.user_settings();
        settings.apply_graphics_recommendation(Recommendation {
            preset: VideoPreset::Quality,
            render_distance: 8,
        });
        assert_eq!(settings.user_settings(), expected);
    }
}

#[test]
fn cache_round_trips_rejects_partial_runs_and_evicts_old_adapters() {
    let mut settings = settings();
    for index in 0..5 {
        assert!(settings.cache_graphics_scores(format!("{index:064x}"), scores(&TIERS[2])));
    }
    let loaded = SettingsOptions::decode(&serde_json::to_vec(&settings).unwrap()).unwrap();
    assert!(loaded.graphics_scores(&format!("{:064x}", 0)).is_none());
    assert_eq!(
        loaded.graphics_scores(&format!("{:064x}", 4)),
        Some(&scores(&TIERS[2]))
    );
    assert!(loaded.graphics_scores(&format!("{:064x}", 5)).is_none());
    assert!(!settings.cache_graphics_scores(format!("{:064x}", 5), Scores::default()));
    let mut partial = scores(&TIERS[2]);
    partial.gpu_p99_ns.clear();
    assert!(!settings.cache_graphics_scores(format!("{:064x}", 5), partial));
}
