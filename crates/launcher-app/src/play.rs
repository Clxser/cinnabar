//! Starts the game client the launcher was installed beside.

use std::{path::Path, process::Command};

use launcher::install_layout::InstallLayout;

/// Starts `layout`'s client as an independent process; the launcher may exit while it runs.
///
/// The error is a sentence for the failure screen, naming the executable it tried.
pub fn start(layout: &InstallLayout) -> Result<(), String> {
    spawn(&layout.client_executable())
}

fn spawn(executable: &Path) -> Result<(), String> {
    if !executable.is_file() {
        return Err(format!(
            "The game was not found at {}. Reinstall {} to restore it.",
            executable.display(),
            launcher::PRODUCT_NAME
        ));
    }
    Command::new(executable)
        .spawn()
        .map(drop)
        .map_err(|error| format!("{} could not start: {error}.", executable.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_client_names_where_it_was_expected() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("cinnabar");
        let message = spawn(&missing).unwrap_err();
        assert!(
            message.contains(&missing.display().to_string()),
            "{message}"
        );
    }
}
