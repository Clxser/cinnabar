//! Worker-only media-tool discovery, including Windows PATH changes made after app startup.

use std::{ffi::OsStr, path::PathBuf};

const NAME: &str = if cfg!(windows) {
    "ffmpeg.exe"
} else {
    "ffmpeg"
};

/// An explicit override is authoritative; an invalid override must not silently select another tool.
pub(super) fn find() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CINNABAR_FFMPEG") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path).ok_or_else(|| {
            "CINNABAR_FFMPEG must point to the FFmpeg executable, not its folder.".into()
        });
    }
    if let Some(path) = std::env::var_os("PATH").as_deref().and_then(on_path) {
        return Ok(path);
    }
    if let Some(path) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|parent| parent.join(NAME)))
        .filter(|path| path.is_file())
    {
        return Ok(path);
    }
    #[cfg(windows)]
    if let Some(path) = windows::installed() {
        return Ok(path);
    }
    Err("FFmpeg was not found. Install FFmpeg, place it beside Cinnabar, or set CINNABAR_FFMPEG to its executable path.".into())
}

/// Searches the caller's directory list without invoking a shell or printing environment values.
fn on_path(value: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(value)
        .map(|directory| directory.join(NAME))
        .find(|path| path.is_file())
}

#[cfg(windows)]
mod windows {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, ptr};

    use windows_sys::{
        Win32::System::Registry::{
            HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
            RegGetValueW,
        },
        core::w,
    };

    /// Fresh registry values cover an installer updating PATH while the launcher remains running.
    pub(super) fn installed() -> Option<super::PathBuf> {
        path_value(HKEY_CURRENT_USER, w!("Environment"))
            .as_deref()
            .and_then(super::on_path)
            .or_else(|| {
                path_value(
                    HKEY_LOCAL_MACHINE,
                    w!("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment"),
                )
                .as_deref()
                .and_then(super::on_path)
            })
    }

    /// Reads a bounded expanded string from a predefined key; a concurrent installer can be retried.
    fn path_value(key: HKEY, subkey: *const u16) -> Option<OsString> {
        const MAX_PATH_UNITS: usize = 32_768;
        let mut buffer = vec![0_u16; MAX_PATH_UNITS];
        let mut bytes = (buffer.len() * size_of::<u16>()) as u32;
        // SAFETY: predefined keys and static strings remain valid; the writable buffer matches bytes.
        let status = unsafe {
            RegGetValueW(
                key,
                subkey,
                w!("Path"),
                RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
                ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status != 0 || bytes as usize > buffer.len() * size_of::<u16>() {
            return None;
        }
        let units = bytes as usize / size_of::<u16>();
        let end = buffer[..units].iter().position(|&unit| unit == 0)?;
        Some(OsString::from_wide(&buffer[..end]))
    }
}
