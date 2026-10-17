use {super::*, client_presentation::camera::FlyCamera};

#[test]
fn shadow_debug_cycles_all_views_and_returns_to_normal_lighting() {
    let mut mode = EnhancedShadowDebug::Off;
    let mut observed = Vec::new();
    for _ in 0..6 {
        mode = next_shadow_debug(mode).0;
        observed.push(mode);
    }
    assert_eq!(
        observed,
        [
            EnhancedShadowDebug::Cascades,
            EnhancedShadowDebug::Visibility,
            EnhancedShadowDebug::DepthNear,
            EnhancedShadowDebug::DepthMiddle,
            EnhancedShadowDebug::DepthFar,
            EnhancedShadowDebug::Off,
        ]
    );
}

#[test]
fn shadow_debug_input_respects_render_mode_focus_and_menu() {
    assert!(!shadow_debug_input_allowed(
        RenderMode::Vanilla,
        true,
        false
    ));
    assert!(!shadow_debug_input_allowed(
        RenderMode::Enhanced,
        false,
        false
    ));
    assert!(!shadow_debug_input_allowed(
        RenderMode::Enhanced,
        true,
        true
    ));
    assert_eq!(
        shadow_debug_input_allowed(RenderMode::Enhanced, true, false),
        enhanced_rendering_enabled(),
    );
}

#[cfg(feature = "enhanced")]
#[test]
fn shadow_key_changes_only_focused_enhanced_gameplay_cameras() {
    let mut app = App::new();
    app.init_resource::<RuntimeSettings>()
        .insert_resource(client_state::PlayerRuntime(player_state::PlayerState::new(
            0,
        )))
        .init_resource::<ButtonInput<KeyCode>>()
        .add_systems(Update, cycle_enhanced_shadow_debug);
    let camera = app
        .world_mut()
        .spawn((FlyCamera::default(), EnhancedRendering::default()))
        .id();
    let window = app
        .world_mut()
        .spawn((Window::default(), bevy::window::PrimaryWindow))
        .id();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(ENHANCED_SHADOW_KEY);
    app.update();
    assert_eq!(
        app.world()
            .get::<EnhancedRendering>(camera)
            .unwrap()
            .shadow_debug,
        EnhancedShadowDebug::Off,
    );
    set_render_mode(
        &mut app.world_mut().resource_mut::<RuntimeSettings>(),
        RenderMode::Enhanced,
    );
    app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
    app.update();
    assert_eq!(
        app.world()
            .get::<EnhancedRendering>(camera)
            .unwrap()
            .shadow_debug,
        EnhancedShadowDebug::Off,
    );
    app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
    app.update();
    assert_eq!(
        app.world()
            .get::<EnhancedRendering>(camera)
            .unwrap()
            .shadow_debug,
        EnhancedShadowDebug::Cascades,
    );
}

/// A Vanilla fallback on an unsupported device keeps server time; `]` sets no override.
#[test]
fn unsupported_adapter_ignores_the_time_shortcut_and_clears_overrides() {
    let mut app = App::new();
    app.init_resource::<RuntimeSettings>()
        .insert_resource(client_state::PlayerRuntime(player_state::PlayerState::new(
            0,
        )))
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<DebugTimeOverride>()
        .insert_resource(render::EnhancedRenderSupport(false))
        .add_systems(Update, cycle_enhanced_time);
    app.world_mut()
        .spawn((Window::default(), bevy::window::PrimaryWindow));
    set_render_mode(
        &mut app.world_mut().resource_mut::<RuntimeSettings>(),
        RenderMode::Enhanced,
    );
    app.world_mut().resource_mut::<DebugTimeOverride>().ticks = Some(6_000);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(ENHANCED_TIME_KEY);
    app.update();
    assert_eq!(app.world().resource::<DebugTimeOverride>().ticks, None);
}

#[test]
fn enhanced_time_presets_cycle_through_daylight_and_server_clock() {
    let mut index = 0;
    let mut values = Vec::new();
    for _ in 0..ENHANCED_TIME_PRESETS.len() {
        values.push(ENHANCED_TIME_PRESETS[index]);
        index = (index + 1) % ENHANCED_TIME_PRESETS.len();
    }
    assert_eq!(
        values,
        [
            (Some(1_000), "Morning"),
            (Some(6_000), "Noon"),
            (Some(9_000), "Afternoon"),
            (Some(12_000), "Sunset"),
            (Some(12_500), "Twilight"),
            (Some(15_000), "Night"),
            (Some(18_000), "Midnight"),
            (None, "Server"),
        ]
    );
}

/// Startup requests cannot enable Enhanced in a default build.
#[cfg(not(feature = "enhanced"))]
#[test]
fn disabled_enhanced_ignores_cli_environment_and_saved_settings() {
    for cli in [None, Some(RenderMode::Vanilla), Some(RenderMode::Enhanced)] {
        for env in [
            None,
            Some(OsStr::new("enhanced")),
            Some(OsStr::new("vanilla")),
        ] {
            for saved in [None, Some(RenderMode::Vanilla), Some(RenderMode::Enhanced)] {
                for attributable in [false, true] {
                    assert_eq!(
                        startup_render_mode(cli, env, saved, attributable),
                        RenderMode::Vanilla,
                    );
                }
            }
        }
    }
}

/// The opt-in feature permits explicit Enhanced requests while evidence runs stay vanilla.
#[cfg(feature = "enhanced")]
#[test]
fn enhanced_feature_respects_startup_precedence() {
    assert_eq!(
        startup_render_mode(
            Some(RenderMode::Enhanced),
            Some(OsStr::new("vanilla")),
            Some(RenderMode::Vanilla),
            true,
        ),
        RenderMode::Enhanced,
    );
    assert_eq!(
        startup_render_mode(
            None,
            Some(OsStr::new("enhanced")),
            Some(RenderMode::Vanilla),
            false,
        ),
        RenderMode::Enhanced,
    );
    assert_eq!(
        startup_render_mode(None, None, Some(RenderMode::Enhanced), false),
        RenderMode::Enhanced,
    );
    assert_eq!(
        startup_render_mode(
            None,
            Some(OsStr::new("enhanced")),
            Some(RenderMode::Enhanced),
            true,
        ),
        RenderMode::Vanilla,
    );
}

#[test]
fn saved_render_mode_round_trips_and_rejects_malformed_files() {
    let directory = std::env::temp_dir().join(format!(
        "cinnabar-render-mode-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let path = directory.join("nested/graphics.json");
    assert_eq!(load_render_mode(&path), None);
    save_render_mode(&path, RenderMode::Enhanced).expect("save");
    assert_eq!(load_render_mode(&path), Some(RenderMode::Enhanced));
    save_render_mode(&path, RenderMode::Vanilla).expect("save");
    assert_eq!(load_render_mode(&path), Some(RenderMode::Vanilla));
    fs::write(&path, br#"{"render_mode":"ultra"}"#).expect("write");
    assert_eq!(load_render_mode(&path), None);
    fs::write(&path, vec![b' '; MAX_GRAPHICS_FILE_BYTES as usize + 1]).expect("write");
    assert_eq!(load_render_mode(&path), None);
    let _ = fs::remove_dir_all(directory);
}

/// Stale settings and camera components cannot enable the disabled renderer.
#[cfg(not(feature = "enhanced"))]
#[test]
fn disabled_enhanced_clears_camera_effects_and_rejects_runtime_requests() {
    let mut app = App::new();
    app.init_resource::<RuntimeSettings>().add_systems(
        Update,
        (apply_render_mode_to_cameras, sync_enhanced_bloom).chain(),
    );
    let original = Camera3d::default().depth_texture_usages;
    let camera = app
        .world_mut()
        .spawn((
            Camera3d {
                depth_texture_usages: (TextureUsages::from(original)
                    | TextureUsages::TEXTURE_BINDING)
                    .into(),
                ..default()
            },
            FlyCamera::default(),
            EnhancedRendering::default(),
            Hdr,
            Bloom::default(),
            VanillaDepthUsage(original),
        ))
        .id();
    // Bypass the normal setting setter to simulate stale in-memory state.
    let mut stale = ui::UserSettings::default();
    stale.video.render_mode = RenderMode::Enhanced;
    app.world_mut()
        .resource_mut::<RuntimeSettings>()
        .replace_user_settings(stale);
    app.update();
    assert!(app.world().get::<EnhancedRendering>(camera).is_none());
    assert!(app.world().get::<Hdr>(camera).is_none());
    assert!(app.world().get::<Bloom>(camera).is_none());
    assert!(app.world().get::<VanillaDepthUsage>(camera).is_none());
    assert_eq!(
        TextureUsages::from(
            app.world()
                .get::<Camera3d>(camera)
                .unwrap()
                .depth_texture_usages
        ),
        TextureUsages::from(original),
    );
    set_render_mode(
        &mut app.world_mut().resource_mut::<RuntimeSettings>(),
        RenderMode::Enhanced,
    );
    app.update();
    assert_eq!(
        app.world()
            .resource::<RuntimeSettings>()
            .user_settings_update()
            .1
            .video
            .render_mode,
        RenderMode::Vanilla
    );
    assert!(app.world().get::<EnhancedRendering>(camera).is_none());
    assert!(app.world().get::<Hdr>(camera).is_none());
    assert!(app.world().get::<Bloom>(camera).is_none());
}

#[cfg(feature = "enhanced")]
#[test]
fn unsupported_adapter_keeps_requested_enhanced_cameras_vanilla() {
    let mut app = App::new();
    app.init_resource::<RuntimeSettings>()
        .insert_resource(render::EnhancedRenderSupport(false))
        .add_systems(Update, apply_render_mode_to_cameras);
    let camera = app
        .world_mut()
        .spawn((Camera3d::default(), FlyCamera::default()))
        .id();
    set_render_mode(
        &mut app.world_mut().resource_mut::<RuntimeSettings>(),
        RenderMode::Enhanced,
    );
    app.update();
    assert!(app.world().get::<EnhancedRendering>(camera).is_none());
    assert!(app.world().get::<Hdr>(camera).is_none());
}

/// Explicit Enhanced mode adds camera effects and Vanilla restores the original camera.
#[cfg(feature = "enhanced")]
#[test]
fn enhanced_mode_applies_and_clears_camera_effects() {
    let mut app = App::new();
    app.init_resource::<RuntimeSettings>().add_systems(
        Update,
        (apply_render_mode_to_cameras, sync_enhanced_bloom).chain(),
    );
    let original = Camera3d::default().depth_texture_usages;
    let camera = app
        .world_mut()
        .spawn((Camera3d::default(), FlyCamera::default()))
        .id();

    set_render_mode(
        &mut app.world_mut().resource_mut::<RuntimeSettings>(),
        RenderMode::Enhanced,
    );
    app.update();
    assert!(app.world().get::<EnhancedRendering>(camera).is_some());
    assert!(app.world().get::<Hdr>(camera).is_some());
    assert!(app.world().get::<Bloom>(camera).is_some());
    let enhanced_depth = TextureUsages::from(
        app.world()
            .get::<Camera3d>(camera)
            .unwrap()
            .depth_texture_usages,
    );
    assert_eq!(
        enhanced_depth,
        TextureUsages::from(original) | TextureUsages::TEXTURE_BINDING,
    );

    set_render_mode(
        &mut app.world_mut().resource_mut::<RuntimeSettings>(),
        RenderMode::Vanilla,
    );
    app.update();
    assert!(app.world().get::<EnhancedRendering>(camera).is_none());
    assert!(app.world().get::<Hdr>(camera).is_none());
    assert!(app.world().get::<Bloom>(camera).is_none());
    assert_eq!(
        TextureUsages::from(
            app.world()
                .get::<Camera3d>(camera)
                .unwrap()
                .depth_texture_usages
        ),
        TextureUsages::from(original),
    );
}

#[cfg(feature = "enhanced-diagnostics")]
#[test]
fn diagnostic_quality_changes_keep_cameras_within_the_launch_budget() {
    render_model::enable_enhanced_diagnostics(render_model::ENHANCED_DIAGNOSTIC_MAX_VIEWPORT)
        .unwrap();
    let mut app = App::new();
    app.init_resource::<RuntimeSettings>()
        .add_systems(Update, apply_render_mode_to_cameras);
    let camera = app
        .world_mut()
        .spawn((Camera3d::default(), FlyCamera::default()))
        .id();
    for quality in [
        EnhancedQuality::Performance,
        EnhancedQuality::Balanced,
        EnhancedQuality::Ultra,
    ] {
        set_graphics_settings(
            &mut app.world_mut().resource_mut::<RuntimeSettings>(),
            &GraphicsSettings {
                mode: RenderMode::Enhanced,
                quality,
                shader_pack: None,
            },
        );
        app.update();
        let current = app.world().get::<EnhancedRendering>(camera).unwrap();
        let bounded = EnhancedRendering::for_quality(quality).bounded_diagnostic();
        assert_eq!(current.quality, quality);
        assert!(current.shadow_resolution <= bounded.shadow_resolution);
        assert!(current.shadow_distance <= bounded.shadow_distance);
        assert!(!current.waving);
    }
}

fn depth_usage(app: &App, camera: Entity) -> TextureUsages {
    TextureUsages::from(
        app.world()
            .get::<Camera3d>(camera)
            .unwrap()
            .depth_texture_usages,
    )
}

fn shader_pack_status_ready() -> ShaderPackStatus {
    ShaderPackStatus::Ready(PathBuf::from("Dusk.zip"))
}

/// A pack opts cameras in only once ready, a failure falls back, and Vanilla restores depth.
#[cfg(feature = "shaderpacks")]
#[test]
fn shader_pack_cameras_wait_for_a_ready_pack_and_restore_vanilla() {
    let mut app = App::new();
    app.init_resource::<RuntimeSettings>()
        .add_systems(Update, apply_render_mode_to_cameras);
    let original = TextureUsages::from(Camera3d::default().depth_texture_usages);
    let camera = app
        .world_mut()
        .spawn((Camera3d::default(), FlyCamera::default()))
        .id();
    let opted_in = |app: &App| app.world().get::<ShaderPackRendering>(camera).is_some();
    set_render_mode(
        &mut app.world_mut().resource_mut::<RuntimeSettings>(),
        RenderMode::ShaderPack,
    );
    app.world_mut()
        .insert_resource(ShaderPackStatus::Loading(PathBuf::from("Dusk.zip")));
    app.update();
    assert!(!opted_in(&app));
    assert_eq!(depth_usage(&app, camera), original);

    app.world_mut().insert_resource(shader_pack_status_ready());
    app.update();
    assert!(opted_in(&app));
    assert!(app.world().get::<EnhancedRendering>(camera).is_none());
    assert_eq!(
        depth_usage(&app, camera),
        original | TextureUsages::TEXTURE_BINDING
    );

    app.world_mut().insert_resource(ShaderPackStatus::Failed {
        pack: PathBuf::from("Dusk.zip"),
        reason: "no shaders folder".to_owned(),
    });
    app.update();
    assert!(!opted_in(&app));
    assert_eq!(depth_usage(&app, camera), original);

    app.world_mut().insert_resource(shader_pack_status_ready());
    app.update();
    assert!(opted_in(&app));
    set_render_mode(
        &mut app.world_mut().resource_mut::<RuntimeSettings>(),
        RenderMode::Vanilla,
    );
    app.update();
    assert!(!opted_in(&app));
    assert!(app.world().get::<VanillaDepthUsage>(camera).is_none());
    assert_eq!(depth_usage(&app, camera), original);
}

/// Switching between the optional renderers never leaves a camera in both.
#[cfg(all(feature = "enhanced", feature = "shaderpacks"))]
#[test]
fn switching_between_optional_renderers_never_keeps_both() {
    let mut app = App::new();
    app.init_resource::<RuntimeSettings>()
        .insert_resource(shader_pack_status_ready())
        .add_systems(Update, apply_render_mode_to_cameras);
    let original = TextureUsages::from(Camera3d::default().depth_texture_usages);
    let camera = app
        .world_mut()
        .spawn((Camera3d::default(), FlyCamera::default()))
        .id();
    for (mode, enhanced, shader_pack) in [
        (RenderMode::Enhanced, true, false),
        (RenderMode::ShaderPack, false, true),
        (RenderMode::Enhanced, true, false),
    ] {
        set_render_mode(&mut app.world_mut().resource_mut::<RuntimeSettings>(), mode);
        app.update();
        assert_eq!(
            app.world().get::<EnhancedRendering>(camera).is_some(),
            enhanced
        );
        assert_eq!(
            app.world().get::<ShaderPackRendering>(camera).is_some(),
            shader_pack
        );
        assert_eq!(
            depth_usage(&app, camera),
            original | TextureUsages::TEXTURE_BINDING
        );
    }
    set_render_mode(
        &mut app.world_mut().resource_mut::<RuntimeSettings>(),
        RenderMode::Vanilla,
    );
    app.update();
    assert_eq!(depth_usage(&app, camera), original);
}

/// Stale settings and a ready pack cannot opt cameras into the disabled renderer.
#[cfg(not(feature = "shaderpacks"))]
#[test]
fn disabled_shader_packs_clear_stale_camera_opt_ins() {
    let mut app = App::new();
    app.init_resource::<RuntimeSettings>()
        .insert_resource(shader_pack_status_ready())
        .add_systems(Update, apply_render_mode_to_cameras);
    let original = Camera3d::default().depth_texture_usages;
    let camera = app
        .world_mut()
        .spawn((
            Camera3d {
                depth_texture_usages: (TextureUsages::from(original)
                    | TextureUsages::TEXTURE_BINDING)
                    .into(),
                ..default()
            },
            FlyCamera::default(),
            ShaderPackRendering,
            VanillaDepthUsage(original),
        ))
        .id();
    let mut stale = ui::UserSettings::default();
    stale.video.render_mode = RenderMode::ShaderPack;
    stale.video.shader_pack = Some("Dusk.zip".to_owned());
    app.world_mut()
        .resource_mut::<RuntimeSettings>()
        .replace_user_settings(stale);
    app.update();
    assert!(app.world().get::<ShaderPackRendering>(camera).is_none());
    assert_eq!(depth_usage(&app, camera), TextureUsages::from(original));
}

#[test]
fn a_failed_shader_pack_draws_vanilla() {
    let failed = ShaderPackStatus::Failed {
        pack: PathBuf::from("Dusk.zip"),
        reason: "no shaders folder".to_owned(),
    };
    assert_eq!(
        effective_render_mode(RenderMode::ShaderPack, None, Some(&failed)),
        RenderMode::Vanilla
    );
    let loading = ShaderPackStatus::Loading(PathBuf::from("Dusk.zip"));
    let compiled = if SHADER_PACKS_ENABLED {
        RenderMode::ShaderPack
    } else {
        RenderMode::Vanilla
    };
    assert_eq!(
        effective_render_mode(RenderMode::ShaderPack, None, Some(&loading)),
        compiled
    );
    assert_eq!(
        effective_render_mode(RenderMode::ShaderPack, None, None),
        compiled
    );
}

/// Startup selects Shader pack only in builds that compile it; evidence runs stay Vanilla.
#[test]
fn shader_pack_startup_follows_the_feature_and_evidence_runs() {
    let compiled = if SHADER_PACKS_ENABLED {
        RenderMode::ShaderPack
    } else {
        RenderMode::Vanilla
    };
    assert_eq!(
        startup_render_mode(Some(RenderMode::ShaderPack), None, None, true),
        compiled
    );
    assert_eq!(
        startup_render_mode(None, Some(OsStr::new("shaderpack")), None, false),
        compiled
    );
    assert_eq!(
        startup_render_mode(None, None, Some(RenderMode::ShaderPack), false),
        compiled
    );
    assert_eq!(
        startup_render_mode(
            None,
            Some(OsStr::new("shaderpack")),
            Some(RenderMode::ShaderPack),
            true,
        ),
        RenderMode::Vanilla
    );
}

#[test]
fn saved_shader_pack_round_trips_and_older_files_still_load() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("graphics.json");
    fs::write(&path, br#"{"render_mode":"enhanced"}"#).unwrap();
    assert_eq!(
        load_graphics_settings(&path),
        Some(GraphicsSettings {
            mode: RenderMode::Enhanced,
            quality: EnhancedQuality::default(),
            shader_pack: None,
        })
    );
    let saved = GraphicsSettings {
        mode: RenderMode::ShaderPack,
        quality: EnhancedQuality::Ultra,
        shader_pack: Some("Dusk.zip".to_owned()),
    };
    save_graphics_settings(&path, &saved).unwrap();
    assert_eq!(load_graphics_settings(&path), Some(saved));
    save_render_mode(&path, RenderMode::Vanilla).unwrap();
    assert_eq!(
        load_graphics_settings(&path).and_then(|saved| saved.shader_pack),
        Some("Dusk.zip".to_owned()),
        "switching renderer keeps the selected pack"
    );
}
