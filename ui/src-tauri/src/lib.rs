//! Tauri backend for the full MTX Panel app (PROJECT_SPEC.md Section 7,
//! build-order stage 7). Talks to the daemon over D-Bus — no DSP or
//! PipeWire code belongs in this crate; it's a thin client, same as the
//! Quickshell widget.

mod commands;
mod daemon;
mod settings;

use daemon::DaemonState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let state = DaemonState::connect()?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_groups,
            commands::list_streams,
            commands::create_group,
            commands::delete_group,
            commands::route_stream_to_group,
            commands::unroute_stream,
            commands::set_gain,
            commands::set_master_gain,
            commands::get_master_gain,
            commands::set_eq_band,
            commands::set_highpass,
            commands::set_lowpass,
            commands::set_compressor,
            commands::set_gate,
            commands::get_group_details,
            commands::get_levels,
            commands::get_spectrum,
            commands::get_metrics,
            commands::set_stream_muted,
            settings::get_autostart,
            settings::set_autostart,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
