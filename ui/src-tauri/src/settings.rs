//! Settings tab (PROJECT_SPEC.md Section 7.5). Only "autostart" is real
//! backend state so far — everything else on that tab (theme, etc.) is
//! either UI-only or still an open question per the spec's own Section 11.
//!
//! Autostart just toggles the user's own systemd unit; no daemon/D-Bus
//! involvement needed, so this lives here rather than in `commands.rs`.

const UNIT: &str = "mtxpanel-daemon.service";

#[tauri::command]
pub fn get_autostart() -> bool {
    std::process::Command::new("systemctl")
        .args(["--user", "is-enabled", "--quiet", UNIT])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let verb = if enabled { "enable" } else { "disable" };
    let status = std::process::Command::new("systemctl")
        .args(["--user", verb, "--quiet", UNIT])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("systemctl --user {verb} {UNIT} failed"))
    }
}
