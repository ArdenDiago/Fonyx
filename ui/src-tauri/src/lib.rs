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
            commands::list_outputs,
            commands::get_output_latency,
            commands::list_group_output_routes,
            commands::set_group_output_route,
            commands::remove_group_output_route,
            commands::set_output_synchronization,
            commands::set_max_compensation_ms,
            commands::get_max_compensation_ms,
            commands::list_scenes,
            commands::capture_current_as_scene,
            commands::update_scene,
            commands::rename_scene,
            commands::duplicate_scene,
            commands::delete_scene,
            commands::activate_scene,
            commands::list_profiles,
            commands::save_profile_for_stream,
            commands::apply_profile_to_app,
            commands::delete_profile,
            commands::list_rules,
            commands::create_auto_route_rule,
            commands::create_device_connected_rule,
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
