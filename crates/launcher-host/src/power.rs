//! Installed power-source information used only when choosing automatic graphics defaults.

/// Reports battery or AC operation, retaining unknown platform data as absent.
pub fn on_battery() -> Option<bool> {
    read_on_battery()
}

/// Reads the macOS power source without starting a shell or requesting user input.
#[cfg(target_os = "macos")]
fn read_on_battery() -> Option<bool> {
    let output = std::process::Command::new("/usr/bin/pmset")
        .args(["-g", "batt"])
        .output()
        .ok()?;
    let text = std::str::from_utf8(&output.stdout).ok()?;
    if !output.status.success() {
        return None;
    }
    if text.contains("AC Power") {
        Some(false)
    } else if text.contains("Battery Power") {
        Some(true)
    } else {
        None
    }
}

/// Reads the current AC state reported by the Windows power manager.
#[cfg(target_os = "windows")]
fn read_on_battery() -> Option<bool> {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    // SAFETY: every field in this Windows output structure admits zero.
    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    // SAFETY: the API writes only to this initialized, correctly sized stack value.
    if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
        return None;
    }
    match status.ACLineStatus {
        0 => Some(true),
        1 => Some(false),
        _ => None,
    }
}

/// Checks Linux power-supply entries, treating desktops without a battery as AC powered.
#[cfg(target_os = "linux")]
fn read_on_battery() -> Option<bool> {
    let mut battery = false;
    for entry in std::fs::read_dir("/sys/class/power_supply").ok()? {
        let path = entry.ok()?.path();
        let kind = std::fs::read_to_string(path.join("type")).ok()?;
        if kind.trim() == "Battery" {
            battery = true;
        }
        if let Ok(online) = std::fs::read_to_string(path.join("online"))
            && online.trim() == "1"
        {
            return Some(false);
        }
    }
    Some(battery)
}

/// Leaves power unknown on platforms without a native reader.
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn read_on_battery() -> Option<bool> {
    None
}

/// Metal drivers ship with macOS; its build identity invalidates otherwise empty driver keys.
pub fn graphics_driver_build() -> String {
    #[cfg(target_os = "macos")]
    {
        return std::process::Command::new("/usr/bin/sw_vers")
            .arg("-buildVersion")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_default();
    }
    #[cfg(not(target_os = "macos"))]
    String::new()
}
