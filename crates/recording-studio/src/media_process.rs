//! Native media helpers share one platform-specific child-process policy.

use std::process::Command;

mod child;
pub(crate) use child::MediaChild;

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
