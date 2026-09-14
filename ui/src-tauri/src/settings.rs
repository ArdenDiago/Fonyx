//! Settings tab (PROJECT_SPEC.md Section 7.5). "Autostart" (daemon and
//! Quickshell widget) is real backend state; everything else on that tab
//! either lives in the daemon's own Config (device selection, buffer size
//! — see `commands.rs`) or is UI-only (theme).
//!
//! Both toggles just enable/disable the user's own systemd unit; no
//! daemon/D-Bus involvement needed, so this lives here rather than in
//! `commands.rs`.

const DAEMON_UNIT: &str = "mtxpanel-daemon.service";
const QUICKSHELL_UNIT: &str = "mtxpanel-quickshell.service";

fn is_enabled(unit: &str) -> bool {
    std::process::Command::new("systemctl")
        .args(["--user", "is-enabled", "--quiet", unit])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn set_enabled(unit: &str, enabled: bool) -> Result<(), String> {
    let verb = if enabled { "enable" } else { "disable" };
    let status = std::process::Command::new("systemctl")
        .args(["--user", verb, "--quiet", unit])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("systemctl --user {verb} {unit} failed"))
    }
}

#[tauri::command]
pub fn get_autostart() -> bool {
    is_enabled(DAEMON_UNIT)
}

#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    set_enabled(DAEMON_UNIT, enabled)
}

#[tauri::command]
pub fn get_quickshell_autostart() -> bool {
    is_enabled(QUICKSHELL_UNIT)
}

#[tauri::command]
pub fn set_quickshell_autostart(enabled: bool) -> Result<(), String> {
    set_enabled(QUICKSHELL_UNIT, enabled)
}
