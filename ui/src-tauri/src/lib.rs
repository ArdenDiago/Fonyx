//! Tauri backend for the full Fonyx app (PROJECT_SPEC.md Section 7,
//! build-order stage 7). Talks to the daemon over D-Bus — no DSP or
//! PipeWire code belongs in this crate; it's a thin client, same as the
//! Quickshell widget.

mod commands;
mod daemon;
mod settings;

use daemon::DaemonState;
use tauri::Manager;

/// The webview keeps its local storage (fader values, mute/solo, custom
/// presets) in a directory named after the app identifier, which changed
/// with the project's name. Move the old one across once; never overwrite.
fn adopt_old_data_dir() {
    let Some(base) = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))
    else {
        return;
    };
    let new = base.join("org.fonyx.app");
    for old in ["org.sonicforge.app", "org.mtxpanel.app"] { // newest first
        let old = base.join(old);
        if old.is_dir() && !new.exists() {
            let _ = std::fs::rename(old, &new);
        }
    }
}

/// The AppImage carries the daemon next to the window. Start it when nothing
/// else owns the bus name, and stop it with the window, so trying Fonyx out
/// leaves nothing running. An installed daemon (systemd unit, snap) is left
/// alone, and outside an AppImage this does nothing.
struct BundledDaemon(std::sync::Mutex<Option<std::process::Child>>);

fn start_bundled_daemon() -> Result<Option<std::process::Child>, String> {
    if std::env::var_os("APPIMAGE").is_none() {
        return Ok(None);
    }
    let bus = zbus::blocking::Connection::session().ok().and_then(|c| zbus::blocking::fdo::DBusProxy::new(&c).ok());
    let env = |k| std::env::var_os(k).map(std::path::PathBuf::from);
    let pipewire_socket = env("PIPEWIRE_RUNTIME_DIR")
        .or_else(|| env("XDG_RUNTIME_DIR"))
        .map(|d| d.join(env("PIPEWIRE_REMOTE").unwrap_or_else(|| "pipewire-0".into())));
    let missing = missing_requirements(bus.is_some(), pipewire_socket.is_some_and(|s| s.exists()), on_path("wpctl"));
    if !missing.is_empty() {
        return Err(format!("Fonyx needs these, and this system is missing them:\n\n{}", missing.join("\n")));
    }
    let bus = bus.unwrap();
    let running = || bus.name_has_owner("org.fonyx.Daemon".try_into().unwrap()).unwrap_or(false);
    if running() {
        return Ok(None);
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name("fonyx-daemon");
    let mut cmd = std::process::Command::new(exe);
    // SIGTERM when the window's process dies, however it dies (a crash, a
    // kill): the daemon runs from the AppImage's mount, which goes with it.
    // SAFETY: prctl is async-signal-safe and touches no memory.
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| format!("Could not start the audio engine: {e}"))?;
    // Let it claim the name before the window's first calls (5 s at most).
    for _ in 0..50 {
        if running() {
            break;
        }
        // A user reported that double-clicking the AppImage did "nothing":
        // a daemon that dies at startup (no PipeWire, say) must say so.
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "The audio engine stopped at startup ({status}).\n\n\
                 Fonyx needs PipeWire with WirePlumber (check with `wpctl status`). \
                 Run the AppImage from a terminal to see the full error."
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(Some(child))
}

/// What the daemon needs from the system that the AppImage cannot carry.
fn missing_requirements(session_bus: bool, pipewire: bool, wpctl: bool) -> Vec<&'static str> {
    [
        (session_bus, "• A D-Bus session bus (log in to a normal desktop session)"),
        (pipewire, "• A running PipeWire (it is not running, or the system uses PulseAudio)"),
        (wpctl, "• WirePlumber's `wpctl` (install the wireplumber package)"),
    ]
    .into_iter()
    .filter(|(ok, _)| !ok)
    .map(|(_, what)| what)
    .collect()
}

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
}

/// Started from a file manager, a failure is otherwise invisible: stderr goes
/// nowhere and the process just exits.
fn startup_error(msg: &str) -> ! {
    eprintln!("fonyx: {msg}");
    if gtk::init().is_ok() {
        use gtk::prelude::*;
        let dialog = gtk::MessageDialog::new(
            None::<&gtk::Window>,
            gtk::DialogFlags::MODAL,
            gtk::MessageType::Error,
            gtk::ButtonsType::Close,
            "Fonyx could not start",
        );
        dialog.set_secondary_text(Some(msg));
        dialog.run();
    }
    std::process::exit(1)
}

/// SIGTERM, not kill(): on SIGTERM the daemon hands every app back to its own
/// output before exiting.
fn stop_bundled_daemon(app: &tauri::AppHandle) {
    if let Some(mut child) = app.state::<BundledDaemon>().0.lock().unwrap().take() {
        // SAFETY: a plain syscall on our own child's pid.
        unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
        let _ = child.wait();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    adopt_old_data_dir();
    daemon::start_snap_daemon();
    tauri::Builder::default()
        .setup(|app| {
            app.manage(BundledDaemon(std::sync::Mutex::new(start_bundled_daemon().unwrap_or_else(|e| startup_error(&e)))));
            let state = DaemonState::connect().unwrap_or_else(|e| startup_error(&format!("No connection to the session bus: {e}")));
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_groups,
            commands::list_streams,
            commands::set_stream_side,
            commands::list_stream_sides,
            commands::get_stream_levels,
            commands::create_group,
            commands::delete_group,
            commands::route_stream_to_group,
            commands::unroute_stream,
            commands::set_gain,
            commands::set_master_gain,
            commands::get_master_gain,
            commands::set_eq_band,
            commands::set_eq_level,
            commands::get_eq_level,
            commands::set_pan,
            commands::list_group_pans,
            commands::set_highpass,
            commands::set_lowpass,
            commands::set_compressor,
            commands::set_gate,
            commands::get_group_details,
            commands::get_levels,
            commands::get_spectrum,
            commands::get_metrics,
            commands::set_stream_muted,
            commands::list_outputs,
            commands::get_output_volume,
            commands::set_output_volume,
            commands::set_output_muted,
            commands::list_group_output_routes,
            commands::set_group_output_route,
            commands::remove_group_output_route,
            commands::set_output_synchronization,
            commands::list_master_output_routes,
            commands::set_master_output_route,
            commands::remove_master_output_route,
            commands::set_master_output_synchronization,
            commands::set_max_compensation_ms,
            commands::get_max_compensation_ms,
            commands::set_cue_output,
            commands::list_cue_outputs,
            commands::get_cue_group_id,
            commands::set_cue_duck_level,
            commands::get_cue_duck_level,
            commands::list_scenes,
            commands::capture_current_as_scene,
            commands::update_scene,
            commands::duplicate_scene,
            commands::delete_scene,
            commands::activate_scene,
            commands::save_profile_for_stream,
            commands::list_group_feeds,
            commands::create_app_strip,
            commands::list_rules,
            commands::get_rule,
            commands::save_rule,
            commands::rule_history,
            commands::set_rule_enabled,
            commands::delete_rule,
            commands::list_ducking_rules,
            commands::create_ducking_rule,
            commands::set_ducking_rule_enabled,
            commands::delete_ducking_rule,
            commands::run_health_checks,
            commands::get_graph,
            commands::start_recording,
            commands::stop_recording,
            commands::list_recordings,
            commands::list_available_plugins,
            commands::list_group_plugins,
            commands::add_group_plugin,
            commands::remove_group_plugin,
            commands::set_group_plugin_bypass,
            commands::set_group_plugin_param,
            commands::reorder_group_plugin,
            settings::get_autostart,
            settings::set_autostart,
            settings::get_quickshell_autostart,
            settings::set_quickshell_autostart,
            commands::set_preferred_default_output,
            commands::get_preferred_default_output,
            commands::set_buffer_ms,
            commands::get_buffer_ms,
        ])
        .build(tauri::generate_context!())
        .unwrap_or_else(|e| startup_error(&e.to_string()))
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                stop_bundled_daemon(app);
            }
        });
}

#[cfg(test)]
mod tests {
    #[test]
    fn missing_requirements_lists_only_what_is_absent() {
        assert!(super::missing_requirements(true, true, true).is_empty());
        let m = super::missing_requirements(true, false, false);
        assert_eq!(m.len(), 2);
        assert!(m[0].contains("PipeWire") && m[1].contains("wpctl"));
    }
}
