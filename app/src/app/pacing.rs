//! Installs frame pacing and connects native Reflex capabilities to Video settings.

use bevy::prelude::{App, First, Last, Update};
use client_shell::menu::MenuRuntime;

/// An explicit launch policy takes precedence over the persisted Video selection.
pub(super) fn selected_mode(
    args: &crate::args::ClientArgs,
    settings: &launcher::menu::settings_options::SettingsOptions,
) -> reflex::Mode {
    if args.reflex_mode_explicit {
        args.reflex_mode
    } else {
        settings.user_settings().video.reflex_mode
    }
}

/// Installs pacing, display-refresh observation and the live Reflex settings row.
pub(super) fn install(app: &mut App, mode: reflex::Mode, cli_override: bool) {
    app.insert_resource(ReflexCliOverride(cli_override))
        .add_plugins(render::InputPacingPlugin::default().with_reflex_mode(mode))
        .add_systems(Update, sync_reflex_menu_capability)
        .init_resource::<crate::present_mode::DisplayRefresh>()
        .add_systems(First, crate::frame_pacing::track_display_refresh)
        .add_systems(Last, crate::frame_pacing::update_frame_pacing);
}

/// Keeps the Video settings' Reflex row aligned with the adapter selected by the renderer.
#[derive(bevy::prelude::Resource, Clone, Copy)]
struct ReflexCliOverride(bool);

fn sync_reflex_menu_capability(
    reflex: Option<bevy::prelude::Res<render::ReflexState>>,
    settings: Option<bevy::prelude::Res<client_state::RuntimeSettings>>,
    cli_override: bevy::prelude::Res<ReflexCliOverride>,
    mut menu: bevy::prelude::ResMut<MenuRuntime>,
) {
    if let Some(reflex) = reflex {
        if !cli_override.0
            && let Some(settings) = settings
        {
            reflex.set_requested_mode(settings.user_settings_update().1.video.reflex_mode);
        }
        menu.set_reflex_gpu_available(reflex.gpu_targets_reflex());
    }
}
