//! Render-mode setting: saved choice, session overrides, and the per-camera
//! Enhanced and shader-pack opt-ins. Vanilla stays the default and the evidence-run mode.

use std::{
    ffi::OsStr,
    fs,
    io::Read as _,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use bevy::{
    camera::{Camera3dDepthTextureUsage, Hdr},
    ecs::system::lifetimeless::{Read, Write},
    post_process::bloom::{Bloom, BloomCompositeMode, BloomPrefilter},
    prelude::*,
    render::render_resource::TextureUsages,
};
use render_model::{SHADER_PACKS_ENABLED, enhanced_rendering_enabled};
use serde::{Deserialize, Serialize};
use ui::RenderMode;
use {
    render::{EnhancedRenderPlugin, ShaderPackRenderPlugin},
    render_api::EnhancedQuality,
    render_scene::{
        EnhancedRendering,
        enhanced::EnhancedShadowDebug,
        shaderpack::{ShaderPackRendering, ShaderPackRequest, ShaderPackStatus},
    },
};

use {
    crate::environment::DebugTimeOverride, client_presentation::camera::FlyCamera,
    client_shell::menu::MenuRuntime, client_state::RuntimeSettings,
};

pub(crate) mod authored_textures;
mod shader_pack;

#[cfg(all(test, feature = "enhanced"))]
mod input_tests;

pub(crate) const RENDER_MODE_ENV: &str = "CINNABAR_RENDER_MODE";
const MAX_GRAPHICS_FILE_BYTES: u64 = 4096;
// The right bracket is currently unclaimed by the built-in debug, modding, and
// experience controls, so the probe does not change existing shortcuts.
const ENHANCED_TIME_KEY: KeyCode = KeyCode::BracketRight;
const ENHANCED_SHADOW_KEY: KeyCode = KeyCode::BracketLeft;

const ENHANCED_TIME_PRESETS: [(Option<u32>, &str); 8] = [
    (Some(1_000), "Morning"),
    (Some(6_000), "Noon"),
    (Some(9_000), "Afternoon"),
    (Some(12_000), "Sunset"),
    (Some(12_500), "Twilight"),
    (Some(15_000), "Night"),
    (Some(18_000), "Midnight"),
    (None, "Server"),
];

/// Systems that update render-mode state before the shared atmosphere frame.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RenderModeUpdateSet;

#[derive(Serialize, Deserialize)]
struct GraphicsFile {
    render_mode: String,
    #[serde(default)]
    enhanced_quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shader_pack: Option<String>,
}

/// The graphics extension choices saved in `graphics.json`.
#[derive(Clone, Debug, Default, PartialEq)]
struct GraphicsSettings {
    mode: RenderMode,
    quality: EnhancedQuality,
    /// Selected pack name inside the shader-pack directory.
    shader_pack: Option<String>,
}

impl GraphicsSettings {
    /// The choices currently applied in `video`.
    fn of(video: &ui::VideoSettings) -> Self {
        Self {
            mode: video.render_mode,
            quality: video.enhanced_quality,
            shader_pack: video.shader_pack.clone(),
        }
    }
}

/// Missing quality in older files uses Balanced and a missing pack selects none, without
/// losing the mode.
fn load_graphics_settings(path: &Path) -> Option<GraphicsSettings> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_GRAPHICS_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_GRAPHICS_FILE_BYTES {
        return None;
    }
    let file = serde_json::from_slice::<GraphicsFile>(&bytes).ok()?;
    Some(GraphicsSettings {
        mode: RenderMode::parse(&file.render_mode)?,
        quality: file
            .enhanced_quality
            .as_deref()
            .and_then(EnhancedQuality::parse)
            .unwrap_or_default(),
        shader_pack: file.shader_pack.filter(|name| !name.is_empty()),
    })
}

/// Atomically replace the small graphics extension settings file.
fn save_graphics_settings(path: &Path, graphics: &GraphicsSettings) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        Context::with_context(fs::create_dir_all(parent), || {
            format!("create {}", parent.display())
        })?;
    }
    let bytes = serde_json::to_vec_pretty(&GraphicsFile {
        render_mode: graphics.mode.as_str().to_owned(),
        enhanced_quality: Some(graphics.quality.as_str().to_owned()),
        shader_pack: graphics.shader_pack.clone(),
    })?;
    let temp = path.with_extension("json.tmp");
    Context::with_context(fs::write(&temp, bytes), || {
        format!("write {}", temp.display())
    })?;
    Context::with_context(fs::rename(&temp, path), || {
        format!("replace {}", path.display())
    })
}

#[cfg(test)]
fn load_render_mode(path: &Path) -> Option<RenderMode> {
    load_graphics_settings(path).map(|saved| saved.mode)
}

#[cfg(test)]
fn save_render_mode(path: &Path, mode: RenderMode) -> Result<()> {
    let saved = load_graphics_settings(path).unwrap_or_default();
    save_graphics_settings(path, &GraphicsSettings { mode, ..saved })
}

/// The mode itself when this build compiles it, else Vanilla.
fn compiled_render_mode(mode: RenderMode) -> RenderMode {
    match mode {
        RenderMode::Enhanced if enhanced_rendering_enabled() => mode,
        RenderMode::ShaderPack if SHADER_PACKS_ENABLED => mode,
        _ => RenderMode::Vanilla,
    }
}

/// CLI beats environment beats the saved file. Attributable evidence runs
/// ignore the environment and saved file so captures stay vanilla. A mode this
/// build does not compile resolves to Vanilla.
pub(crate) fn startup_render_mode(
    cli: Option<RenderMode>,
    env: Option<&OsStr>,
    saved: Option<RenderMode>,
    attributable: bool,
) -> RenderMode {
    let requested = if cli.is_some() {
        cli
    } else if attributable {
        None
    } else {
        env.and_then(OsStr::to_str)
            .and_then(RenderMode::parse)
            .or(saved)
    };
    compiled_render_mode(requested.unwrap_or_default())
}

pub(crate) struct RenderModePlugin {
    cli: Option<RenderMode>,
    attributable: bool,
    shader_pack: Option<PathBuf>,
}

impl RenderModePlugin {
    /// Configure session overrides and deterministic evidence runs.
    pub(crate) const fn new(cli: Option<RenderMode>, attributable: bool) -> Self {
        Self {
            cli,
            attributable,
            shader_pack: None,
        }
    }

    /// Draws this `--shader-pack` path instead of the saved selection while the session's
    /// mode is Shader pack; it is never saved.
    #[must_use]
    pub(crate) fn with_shader_pack(mut self, shader_pack: Option<PathBuf>) -> Self {
        self.shader_pack = shader_pack;
        self
    }
}

#[derive(Resource)]
struct RenderModeConfig {
    cli: Option<RenderMode>,
    attributable: bool,
    path: Option<PathBuf>,
    quality_override: Option<EnhancedQuality>,
    /// The launch `--shader-pack`, until the player picks a renderer in the menu.
    cli_shader_pack: Option<PathBuf>,
    /// Shader-pack folders, known once the menu supplies the install layout.
    shader_packs: Option<shader_pack::ShaderPackDirs>,
}

impl Plugin for RenderModePlugin {
    fn build(&self, app: &mut App) {
        let quality_override = std::env::var("CINNABAR_ENHANCED_QUALITY")
            .ok()
            .and_then(|value| EnhancedQuality::parse(&value));
        app.add_plugins((EnhancedRenderPlugin, ShaderPackRenderPlugin))
            .insert_resource(RenderModeConfig {
                cli: self.cli,
                attributable: self.attributable,
                path: None,
                quality_override,
                cli_shader_pack: self.shader_pack.clone(),
                shader_packs: None,
            })
            .init_resource::<DebugTimeOverride>()
            .init_resource::<ShaderPackRequest>()
            .add_systems(Startup, seed_render_mode)
            .add_systems(
                Update,
                (
                    shader_pack::refresh_shader_packs,
                    apply_menu_render_mode,
                    shader_pack::sync_shader_pack_request,
                    authored_textures::update_authored_textures,
                    apply_render_mode_to_cameras,
                    cycle_enhanced_shadow_debug,
                    sync_enhanced_bloom,
                    cycle_enhanced_time,
                )
                    .chain()
                    .in_set(RenderModeUpdateSet)
                    .after(client_state::ClientFrameSet::UiAuthority),
            );
    }
}

/// Resolve the saved choice and session overrides at startup, and list the shader packs.
fn seed_render_mode(
    mut config: ResMut<RenderModeConfig>,
    mut menu: Option<ResMut<MenuRuntime>>,
    mut settings: ResMut<RuntimeSettings>,
) {
    config.path = menu.as_deref().map(MenuRuntime::graphics_file);
    config.shader_packs = menu
        .as_deref()
        .map(|menu| shader_pack::ShaderPackDirs::new(menu.layout()));
    let saved = config.path.as_deref().and_then(load_graphics_settings);
    let mode = startup_render_mode(
        config.cli,
        std::env::var_os(RENDER_MODE_ENV).as_deref(),
        saved.as_ref().map(|saved| saved.mode),
        config.attributable,
    );
    let quality = if config.attributable {
        EnhancedQuality::default()
    } else {
        config
            .quality_override
            .or(saved.as_ref().map(|saved| saved.quality))
            .unwrap_or_default()
    };
    let pack = saved.and_then(|saved| saved.shader_pack);
    set_graphics_settings(
        &mut settings,
        &GraphicsSettings {
            mode,
            quality,
            shader_pack: pack,
        },
    );
    if let Some(menu) = menu.as_deref_mut() {
        shader_pack::refresh(&mut config, menu);
    }
}

/// Update the shared settings authority only when a choice changes; modes this build does
/// not compile fall back to Vanilla.
fn set_graphics_settings(settings: &mut RuntimeSettings, graphics: &GraphicsSettings) {
    let mode = compiled_render_mode(graphics.mode);
    let (_, current) = settings.user_settings_update();
    if current.video.render_mode != mode
        || current.video.enhanced_quality != graphics.quality
        || current.video.shader_pack != graphics.shader_pack
    {
        let mut next = current.clone();
        next.video.render_mode = mode;
        next.video.enhanced_quality = graphics.quality;
        next.video.shader_pack.clone_from(&graphics.shader_pack);
        settings.replace_user_settings(next);
    }
}

/// Changes render mode while retaining the current quality and pack.
#[cfg(test)]
fn set_render_mode(settings: &mut RuntimeSettings, mode: RenderMode) {
    let graphics = GraphicsSettings {
        mode,
        ..GraphicsSettings::of(&settings.user_settings_update().1.video)
    };
    set_graphics_settings(settings, &graphics);
}

/// Persist menu changes and keep the displayed choices synchronized. A renderer pick ends
/// the `--shader-pack` session override.
fn apply_menu_render_mode(
    mut config: ResMut<RenderModeConfig>,
    menu: Option<ResMut<MenuRuntime>>,
    mut settings: ResMut<RuntimeSettings>,
) {
    let Some(mut menu) = menu else {
        return;
    };
    let mode_request = menu.take_render_mode_request();
    let quality_request = menu.take_enhanced_quality_request();
    let pack_request = menu.take_shader_pack_request();
    if (mode_request.is_some() || pack_request.is_some()) && config.cli_shader_pack.is_some() {
        config.cli_shader_pack = None;
    }
    if mode_request.is_some() || quality_request.is_some() || pack_request.is_some() {
        let current = GraphicsSettings::of(&settings.user_settings_update().1.video);
        let next = GraphicsSettings {
            mode: mode_request.unwrap_or(current.mode),
            quality: quality_request.unwrap_or(current.quality),
            shader_pack: pack_request.or(current.shader_pack),
        };
        set_graphics_settings(&mut settings, &next);
        let applied = GraphicsSettings::of(&settings.user_settings_update().1.video);
        if let Some(path) = &config.path
            && let Err(error) = save_graphics_settings(path, &applied)
        {
            warn!(?error, "graphics settings could not be saved");
        }
    }
    let video = &settings.user_settings_update().1.video;
    menu.sync_render_mode(video.render_mode);
    menu.sync_enhanced_quality(video.enhanced_quality);
    menu.sync_shader_pack(video.shader_pack.as_deref());
}

/// Retains launch-time diagnostic bounds when a camera's quality changes.
fn camera_settings(quality: EnhancedQuality) -> EnhancedRendering {
    let settings = EnhancedRendering::for_quality(quality);
    #[cfg(feature = "enhanced-diagnostics")]
    if render_model::enhanced_diagnostics_enabled() {
        return settings.bounded_diagnostic();
    }
    settings
}

/// Original depth usage, restored once neither optional renderer samples the depth texture.
#[derive(Component)]
struct VanillaDepthUsage(Camera3dDepthTextureUsage);

/// Camera state needed to apply or restore the optional modes.
type RenderModeCameraQuery = (
    Entity,
    Write<Camera3d>,
    Option<Write<EnhancedRendering>>,
    Has<ShaderPackRendering>,
    Option<Read<VanillaDepthUsage>>,
);

/// The mode the renderer actually draws: Enhanced only when compiled in and the device
/// supports it, Shader pack only when compiled in and its pack has not failed, so every
/// optional resource follows the same fallback as the cameras.
pub(crate) fn effective_render_mode(
    requested: RenderMode,
    support: Option<&render::EnhancedRenderSupport>,
    shader_pack: Option<&ShaderPackStatus>,
) -> RenderMode {
    match compiled_render_mode(requested) {
        RenderMode::Enhanced if support.is_some_and(|support| !support.0) => RenderMode::Vanilla,
        RenderMode::ShaderPack if matches!(shader_pack, Some(ShaderPackStatus::Failed { .. })) => {
            RenderMode::Vanilla
        }
        mode => mode,
    }
}

/// Keep the opt-in effects on gameplay cameras only, never both at once. A shader pack
/// opts in only once it is ready; while it loads the camera draws Vanilla.
fn apply_render_mode_to_cameras(
    mut commands: Commands,
    settings: Res<RuntimeSettings>,
    support: Option<Res<render::EnhancedRenderSupport>>,
    status: Option<Res<ShaderPackStatus>>,
    mut cameras: Query<RenderModeCameraQuery, With<FlyCamera>>,
) {
    let video = &settings.user_settings_update().1.video;
    let mode = effective_render_mode(video.render_mode, support.as_deref(), status.as_deref());
    let enhanced = mode == RenderMode::Enhanced;
    let shader_pack =
        mode == RenderMode::ShaderPack && status.as_deref().is_some_and(ShaderPackStatus::is_ready);
    for (entity, mut camera, current_enhanced, has_shader_pack, vanilla_depth) in &mut cameras {
        let mut entity_commands = commands.entity(entity);
        match (enhanced, current_enhanced) {
            (true, None) => {
                entity_commands.insert((camera_settings(video.enhanced_quality), Hdr));
            }
            (true, Some(mut current)) if current.quality != video.enhanced_quality => {
                let shadow_debug = current.shadow_debug;
                *current = EnhancedRendering {
                    shadow_debug,
                    ..camera_settings(video.enhanced_quality)
                };
            }
            (false, Some(_)) => {
                entity_commands.remove::<(
                    EnhancedRendering,
                    bevy::render::camera::TemporalJitter,
                    Hdr,
                    Bloom,
                )>();
            }
            _ => {}
        }
        if shader_pack && !has_shader_pack {
            entity_commands.insert(ShaderPackRendering);
        } else if !shader_pack && has_shader_pack {
            entity_commands.remove::<ShaderPackRendering>();
        }
        // Both optional renderers sample the depth texture.
        match (enhanced || shader_pack, vanilla_depth) {
            (true, None) => {
                let original = camera.depth_texture_usages;
                camera.depth_texture_usages =
                    (TextureUsages::from(original) | TextureUsages::TEXTURE_BINDING).into();
                entity_commands.insert(VanillaDepthUsage(original));
            }
            (false, Some(VanillaDepthUsage(original))) => {
                camera.depth_texture_usages = *original;
                entity_commands.remove::<VanillaDepthUsage>();
            }
            _ => {}
        }
    }
}

/// Apply the per-camera bloom quality switch without changing other effects.
fn sync_enhanced_bloom(
    mut commands: Commands,
    cameras: Query<(Entity, &EnhancedRendering, Has<Bloom>), With<FlyCamera>>,
) {
    for (entity, enhanced, has_bloom) in &cameras {
        let bloom = enhanced_rendering_enabled()
            && enhanced.bloom
            && enhanced.shadow_debug == EnhancedShadowDebug::Off;
        if bloom == has_bloom {
            continue;
        }
        if bloom {
            commands.entity(entity).insert(Bloom {
                intensity: 0.045,
                low_frequency_boost: 0.0,
                prefilter: BloomPrefilter {
                    threshold: 1.5,
                    threshold_softness: 0.25,
                },
                composite_mode: BloomCompositeMode::Additive,
                ..default()
            });
        } else {
            commands.entity(entity).remove::<Bloom>();
        }
    }
}

/// Cycles local lighting presets without changing server time or gameplay.
#[allow(
    clippy::too_many_arguments,
    reason = "Shortcut admission borrows the shared screen owners."
)]
fn cycle_enhanced_time(
    settings: Res<RuntimeSettings>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    menu: Option<Res<MenuRuntime>>,
    player: Res<client_state::PlayerRuntime>,
    ui: Option<Res<client_shell::ui_resources::UiRuntimeResource>>,
    presentation: Option<Res<client_shell::ui_resources::UiPresentationResource>>,
    support: Option<Res<render::EnhancedRenderSupport>>,
    mut debug_time: ResMut<DebugTimeOverride>,
    mut next_preset: Local<usize>,
) {
    // A Vanilla fallback must keep following server time. Shader-pack status never
    // decides whether Enhanced draws.
    let mode = effective_render_mode(
        settings.user_settings_update().1.video.render_mode,
        support.as_deref(),
        None,
    );
    if mode != RenderMode::Enhanced {
        *next_preset = 0;
        debug_time.ticks = None;
        return;
    }
    let focused = windows.iter().next().is_none_or(|window| window.focused);
    if !focused
        || client_shell::screen_policy::absorbs_input(
            &player,
            ui.as_deref().map(|ui| &ui.0),
            menu.as_deref(),
            presentation.as_deref().map(|presentation| &presentation.0),
        )
    {
        return;
    }
    if !keys.just_pressed(ENHANCED_TIME_KEY) {
        return;
    }
    let (ticks, label) = ENHANCED_TIME_PRESETS[*next_preset];
    *next_preset = (*next_preset + 1) % ENHANCED_TIME_PRESETS.len();
    debug_time.ticks = ticks;
    debug!(target: "cinnabar::enhanced", ?ticks, preset = label, "changed debug time preset");
}

/// Admits shadow shortcuts only when focused Enhanced gameplay owns the keyboard.
fn shadow_debug_input_allowed(mode: RenderMode, focused: bool, absorbs_input: bool) -> bool {
    enhanced_rendering_enabled() && mode == RenderMode::Enhanced && focused && !absorbs_input
}

/// Advances through each diagnostic shadow view and back to normal lighting.
fn next_shadow_debug(mode: EnhancedShadowDebug) -> (EnhancedShadowDebug, &'static str) {
    use EnhancedShadowDebug::{Cascades, DepthFar, DepthMiddle, DepthNear, Off, Visibility};
    match mode {
        Off => (Cascades, "Cascade coverage"),
        Cascades => (Visibility, "Shadow visibility"),
        Visibility => (DepthNear, "Near shadow depth"),
        DepthNear => (DepthMiddle, "Middle shadow depth"),
        DepthMiddle => (DepthFar, "Far shadow depth"),
        DepthFar => (Off, "Off"),
    }
}

/// Applies the diagnostic shadow shortcut to active gameplay cameras.
#[allow(
    clippy::too_many_arguments,
    reason = "Shortcut admission borrows the shared screen owners."
)]
fn cycle_enhanced_shadow_debug(
    settings: Res<RuntimeSettings>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    menu: Option<Res<MenuRuntime>>,
    player: Res<client_state::PlayerRuntime>,
    ui: Option<Res<client_shell::ui_resources::UiRuntimeResource>>,
    presentation: Option<Res<client_shell::ui_resources::UiPresentationResource>>,
    support: Option<Res<render::EnhancedRenderSupport>>,
    mut cameras: Query<&mut EnhancedRendering, With<FlyCamera>>,
) {
    let focused = windows.iter().next().is_none_or(|window| window.focused);
    let absorbs_input = client_shell::screen_policy::absorbs_input(
        &player,
        ui.as_deref().map(|ui| &ui.0),
        menu.as_deref(),
        presentation.as_deref().map(|presentation| &presentation.0),
    );
    if !shadow_debug_input_allowed(
        effective_render_mode(
            settings.user_settings_update().1.video.render_mode,
            support.as_deref(),
            None,
        ),
        focused,
        absorbs_input,
    ) || !keys.just_pressed(ENHANCED_SHADOW_KEY)
    {
        return;
    }
    for mut enhanced in &mut cameras {
        let (next, label) = next_shadow_debug(enhanced.shadow_debug);
        enhanced.shadow_debug = next;
        info!(target: "cinnabar::enhanced", mode = label, "changed shadow debug view");
    }
}

#[cfg(test)]
mod tests;
