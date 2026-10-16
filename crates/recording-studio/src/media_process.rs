//! Native media helpers share one platform-specific child-process policy.

use std::process::Command;

mod child;
pub(crate) use child::MediaChild;

/// Flushes an encoder's closed output with write access, which Windows requires for file sync.
pub(crate) fn sync_output(path: &std::path::Path) -> Result<(), String> {
    std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("Cannot flush finished media {}: {error}", path.display()))
}

/// Media encoders, probes and decoders run without opening an extra console on Windows.
pub(crate) fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}
