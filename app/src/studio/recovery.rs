//! Native recovery stays on the existing storage worker and never replaces the original take.

use std::path::Path;

use bevy::prelude::*;
use client_shell::menu::MenuRuntime;
use recording_studio::{edit::Editor, export::Cancellation};

/// Installs only a function pointer; no storage worker, media read or recording starts here.
pub(super) fn install(mut menu: ResMut<MenuRuntime>) {
    menu.set_studio_recording_recovery(recover);
    menu.set_studio_target_coverage(recording_scene::targets::verify_coverage);
}

/// Prefer a verified native prefix; a POV fallback always explains why the native scene was unavailable.
fn recover(path: &Path, cancellation: &Cancellation) -> Result<(Editor, Option<String>), String> {
    cancellation.check()?;
    match recording_scene::recovery::recover_project(path, cancellation) {
        Ok(editor) => {
            let duration = editor.project().duration().seconds();
            Ok((
                editor,
                Some(format!(
                    "Recovered {duration:.3}s of verified native replay into an unsaved project. Only the common committed scene interval is included. Uncommitted tail data, unfinished audio and event markers were not restored. Save this project to retain the edit; the original take is unchanged."
                )),
            ))
        }
        Err(native_error) => {
            cancellation.check()?;
            let info = recording_studio::archive::RecordingInfo::read(path)
                .map_err(|error| error.to_string())?;
            let editor = recording_studio::capture::recover_project(path, &info.name, cancellation)
                .map_err(|pov_error| format!("Native recovery failed: {native_error}\nPOV recovery also failed: {pov_error}"))?;
            Ok((
                editor,
                Some(format!(
                    "Recovered POV video only. Native recovery failed: {native_error}\nScene tracks, unfinished audio, event markers and missing fragments were not restored. Save this project to retain the edit."
                )),
            ))
        }
    }
}
