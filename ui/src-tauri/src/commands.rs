//! Tauri commands: each is a one-line forward to the daemon over D-Bus
//! (`daemon.rs`). No state or logic lives here beyond the connection.
//!
//! All `async` (run on Tauri's thread pool, not the GUI thread) so a slow
//! daemon call can never freeze the window, and they no longer queue
//! behind each other: the proxy is shared lock-free.

use tauri::State;

use crate::daemon::DaemonState;

fn err(e: zbus::Error) -> String {
    e.to_string()
}

#[tauri::command(async)]
pub fn list_groups(state: State<DaemonState>) -> Result<Vec<(u32, String, f32, Vec<u32>, bool)>, String> {
    state.0.list_groups().map_err(err)
}

#[tauri::command(async)]
pub fn list_streams(state: State<DaemonState>) -> Result<Vec<(u32, String, String, bool, String)>, String> {
    state.0.list_streams().map_err(err)
}

#[tauri::command(async)]
pub fn create_group(state: State<DaemonState>, name: String) -> Result<u32, String> {
    state.0.create_group(&name).map_err(err)
}

#[tauri::command(async)]
pub fn delete_group(state: State<DaemonState>, group_id: u32) -> Result<(), String> {
    state.0.delete_group(group_id).map_err(err)
}

#[tauri::command(async)]
pub fn route_stream_to_group(state: State<DaemonState>, stream_id: u32, group_id: u32) -> Result<(), String> {
    state.0.route_stream_to_group(stream_id, group_id).map_err(err)
}

#[tauri::command(async)]
pub fn unroute_stream(state: State<DaemonState>, stream_id: u32) -> Result<(), String> {
    state.0.unroute_stream(stream_id).map_err(err)
}

#[tauri::command(async)]
pub fn set_gain(state: State<DaemonState>, group_id: u32, value: f32) -> Result<(), String> {
    state.0.set_gain(group_id, value).map_err(err)
}

#[tauri::command(async)]
pub fn set_master_gain(state: State<DaemonState>, value: f32) -> Result<(), String> {
    state.0.set_master_gain(value).map_err(err)
}

#[tauri::command(async)]
pub fn get_master_gain(state: State<DaemonState>) -> Result<f32, String> {
    state.0.get_master_gain().map_err(err)
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn set_eq_band(
    state: State<DaemonState>,
    group_id: u32,
    band_index: u32,
    freq: f32,
    gain: f32,
    q: f32,
    enabled: bool,
) -> Result<(), String> {
    state.0.set_eq_band(group_id, band_index, freq, gain, q, enabled).map_err(err)
}

#[tauri::command(async)]
pub fn set_highpass(state: State<DaemonState>, group_id: u32, freq: f32, q: f32, enabled: bool) -> Result<(), String> {
    state.0.set_highpass(group_id, freq, q, enabled).map_err(err)
}

#[tauri::command(async)]
pub fn set_lowpass(state: State<DaemonState>, group_id: u32, freq: f32, q: f32, enabled: bool) -> Result<(), String> {
    state.0.set_lowpass(group_id, freq, q, enabled).map_err(err)
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn set_compressor(
    state: State<DaemonState>,
    group_id: u32,
    threshold_db: f32,
    ratio: f32,
    attack_ms: f32,
    release_ms: f32,
    makeup_gain_db: f32,
    enabled: bool,
) -> Result<(), String> {
    state
        .0
        .set_compressor(group_id, threshold_db, ratio, attack_ms, release_ms, makeup_gain_db, enabled)
        .map_err(err)
}

#[tauri::command(async)]
#[allow(clippy::type_complexity)]
pub fn get_group_details(
    state: State<DaemonState>,
    group_id: u32,
) -> Result<
    (
        Vec<(f32, f32, f32, bool)>,
        (f32, f32, bool),
        (f32, f32, bool),
        (f32, f32, f32, f32, f32, bool),
        (f32, f32, f32, f32, bool),
    ),
    String,
> {
    state.0.get_group_details(group_id).map_err(err)
}

#[tauri::command(async)]
pub fn get_levels(state: State<DaemonState>) -> Result<Vec<(u32, f32, f32, f32, f32)>, String> {
    state.0.get_levels().map_err(err)
}

#[tauri::command(async)]
pub fn get_spectrum(state: State<DaemonState>, group_id: u32) -> Result<Vec<f32>, String> {
    state.0.get_spectrum(group_id).map_err(err)
}

#[tauri::command(async)]
pub fn get_metrics(state: State<DaemonState>) -> Result<(f32, f32, u32, u32), String> {
    state.0.get_metrics().map_err(err)
}

#[tauri::command(async)]
pub fn set_stream_muted(state: State<DaemonState>, stream_id: u32, muted: bool) -> Result<bool, String> {
    state.0.set_stream_muted(stream_id, muted).map_err(err)
}

#[tauri::command(async)]
#[allow(clippy::type_complexity)]
pub fn list_outputs(state: State<DaemonState>) -> Result<Vec<(String, String, String, String, bool, bool, u32, u32)>, String> {
    state.0.list_outputs().map_err(err)
}

#[tauri::command(async)]
pub fn get_output_latency(state: State<DaemonState>, output_id: String) -> Result<(bool, f32), String> {
    state.0.get_output_latency(&output_id).map_err(err)
}

#[tauri::command(async)]
pub fn get_output_volume(state: State<DaemonState>, output_id: String) -> Result<(f32, bool), String> {
    state.0.get_output_volume(&output_id).map_err(err)
}

#[tauri::command(async)]
pub fn set_output_volume(state: State<DaemonState>, output_id: String, volume: f32) -> Result<bool, String> {
    state.0.set_output_volume(&output_id, volume).map_err(err)
}

#[tauri::command(async)]
pub fn set_output_muted(state: State<DaemonState>, output_id: String, muted: bool) -> Result<bool, String> {
    state.0.set_output_muted(&output_id, muted).map_err(err)
}

#[tauri::command(async)]
#[allow(clippy::type_complexity)]
pub fn list_group_output_routes(state: State<DaemonState>, group_id: u32) -> Result<Vec<(String, f32, bool, String, String, f32, bool)>, String> {
    state.0.list_group_output_routes(group_id).map_err(err)
}

#[tauri::command(async)]
pub fn set_group_output_route(state: State<DaemonState>, group_id: u32, output_id: String, gain: f32, enabled: bool) -> Result<(), String> {
    state.0.set_group_output_route(group_id, &output_id, gain, enabled).map_err(err)
}

#[tauri::command(async)]
pub fn remove_group_output_route(state: State<DaemonState>, group_id: u32, output_id: String) -> Result<(), String> {
    state.0.remove_group_output_route(group_id, &output_id).map_err(err)
}

#[tauri::command(async)]
pub fn set_output_synchronization(
    state: State<DaemonState>,
    group_id: u32,
    output_id: String,
    sync_mode: String,
    manual_delay_ms: f32,
) -> Result<(), String> {
    state.0.set_output_synchronization(group_id, &output_id, &sync_mode, manual_delay_ms).map_err(err)
}

#[tauri::command(async)]
#[allow(clippy::type_complexity)]
pub fn list_master_output_routes(state: State<DaemonState>) -> Result<Vec<(String, f32, bool, String, String, f32, bool)>, String> {
    state.0.list_master_output_routes().map_err(err)
}

#[tauri::command(async)]
pub fn set_master_output_route(state: State<DaemonState>, output_id: String, gain: f32, enabled: bool) -> Result<(), String> {
    state.0.set_master_output_route(&output_id, gain, enabled).map_err(err)
}

#[tauri::command(async)]
pub fn remove_master_output_route(state: State<DaemonState>, output_id: String) -> Result<(), String> {
    state.0.remove_master_output_route(&output_id).map_err(err)
}

#[tauri::command(async)]
pub fn set_master_output_synchronization(
    state: State<DaemonState>,
    output_id: String,
    sync_mode: String,
    manual_delay_ms: f32,
) -> Result<(), String> {
    state.0.set_master_output_synchronization(&output_id, &sync_mode, manual_delay_ms).map_err(err)
}

#[tauri::command(async)]
pub fn set_max_compensation_ms(state: State<DaemonState>, value: f32) -> Result<(), String> {
    state.0.set_max_compensation_ms(value).map_err(err)
}

#[tauri::command(async)]
pub fn get_max_compensation_ms(state: State<DaemonState>) -> Result<f32, String> {
    state.0.get_max_compensation_ms().map_err(err)
}

#[tauri::command(async)]
pub fn set_cue_output(state: State<DaemonState>, output_id: String, enabled: bool) -> Result<(), String> {
    state.0.set_cue_output(&output_id, enabled).map_err(err)
}

#[tauri::command(async)]
pub fn list_cue_outputs(state: State<DaemonState>) -> Result<Vec<String>, String> {
    state.0.list_cue_outputs().map_err(err)
}

#[tauri::command(async)]
pub fn get_cue_group_id(state: State<DaemonState>) -> Result<u32, String> {
    state.0.get_cue_group_id().map_err(err)
}

#[tauri::command(async)]
pub fn set_cue_duck_level(state: State<DaemonState>, value: f32) -> Result<(), String> {
    state.0.set_cue_duck_level(value).map_err(err)
}

#[tauri::command(async)]
pub fn get_cue_duck_level(state: State<DaemonState>) -> Result<f32, String> {
    state.0.get_cue_duck_level().map_err(err)
}

#[tauri::command(async)]
pub fn set_preferred_default_output(state: State<DaemonState>, output_id: String) -> Result<(), String> {
    state.0.set_preferred_default_output(&output_id).map_err(err)
}

#[tauri::command(async)]
pub fn get_preferred_default_output(state: State<DaemonState>) -> Result<String, String> {
    state.0.get_preferred_default_output().map_err(err)
}

#[tauri::command(async)]
pub fn set_buffer_ms(state: State<DaemonState>, value: f32) -> Result<(), String> {
    state.0.set_buffer_ms(value).map_err(err)
}

#[tauri::command(async)]
pub fn get_buffer_ms(state: State<DaemonState>) -> Result<f32, String> {
    state.0.get_buffer_ms().map_err(err)
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn set_gate(
    state: State<DaemonState>,
    group_id: u32,
    threshold_db: f32,
    attack_ms: f32,
    release_ms: f32,
    hold_ms: f32,
    enabled: bool,
) -> Result<(), String> {
    state.0.set_gate(group_id, threshold_db, attack_ms, release_ms, hold_ms, enabled).map_err(err)
}

// --- Audio Scenes ---

#[tauri::command(async)]
pub fn list_scenes(state: State<DaemonState>) -> Result<Vec<(u32, String, String)>, String> {
    state.0.list_scenes().map_err(err)
}

#[tauri::command(async)]
pub fn capture_current_as_scene(state: State<DaemonState>, name: String) -> Result<u32, String> {
    state.0.capture_current_as_scene(&name).map_err(err)
}

#[tauri::command(async)]
pub fn update_scene(state: State<DaemonState>, scene_id: u32) -> Result<bool, String> {
    state.0.update_scene(scene_id).map_err(err)
}

#[tauri::command(async)]
pub fn rename_scene(state: State<DaemonState>, scene_id: u32, new_name: String) -> Result<(), String> {
    state.0.rename_scene(scene_id, &new_name).map_err(err)
}

#[tauri::command(async)]
pub fn duplicate_scene(state: State<DaemonState>, scene_id: u32, new_name: String) -> Result<u32, String> {
    state.0.duplicate_scene(scene_id, &new_name).map_err(err)
}

#[tauri::command(async)]
pub fn delete_scene(state: State<DaemonState>, scene_id: u32) -> Result<(), String> {
    state.0.delete_scene(scene_id).map_err(err)
}

#[tauri::command(async)]
pub fn activate_scene(state: State<DaemonState>, scene_id: u32) -> Result<(u32, u32, u32, u32), String> {
    state.0.activate_scene(scene_id).map_err(err)
}

// --- Per-App Audio Profiles ---

#[tauri::command(async)]
#[allow(clippy::type_complexity)]
pub fn list_profiles(state: State<DaemonState>) -> Result<Vec<(String, String, f32, u32, String)>, String> {
    state.0.list_profiles().map_err(err)
}

#[tauri::command(async)]
pub fn save_profile_for_stream(state: State<DaemonState>, stream_id: u32, label: String) -> Result<bool, String> {
    state.0.save_profile_for_stream(stream_id, &label).map_err(err)
}

#[tauri::command(async)]
pub fn apply_profile_to_app(state: State<DaemonState>, app_id: String) -> Result<bool, String> {
    state.0.apply_profile_to_app(&app_id).map_err(err)
}

#[tauri::command(async)]
pub fn delete_profile(state: State<DaemonState>, app_id: String) -> Result<(), String> {
    state.0.delete_profile(&app_id).map_err(err)
}

#[tauri::command(async)]
pub fn set_group_feeds(state: State<DaemonState>, group_id: u32, parent_id: u32) -> Result<(), String> {
    state.0.set_group_feeds(group_id, parent_id).map_err(err)
}

#[tauri::command(async)]
pub fn list_group_feeds(state: State<DaemonState>) -> Result<Vec<(u32, u32)>, String> {
    state.0.list_group_feeds().map_err(err)
}

#[tauri::command(async)]
pub fn create_app_strip(state: State<DaemonState>, stream_id: u32) -> Result<u32, String> {
    state.0.create_app_strip(stream_id).map_err(err)
}

// --- Rules engine ---

#[tauri::command(async)]
#[allow(clippy::type_complexity)]
pub fn list_rules(state: State<DaemonState>) -> Result<Vec<(u32, String, bool, i32, u64, String, String)>, String> {
    state.0.list_rules().map_err(err)
}

#[tauri::command(async)]
pub fn get_rule(state: State<DaemonState>, rule_id: u32) -> Result<String, String> {
    state.0.get_rule(rule_id).map_err(err)
}

#[tauri::command(async)]
pub fn save_rule(state: State<DaemonState>, json: String) -> Result<u32, String> {
    state.0.save_rule(&json).map_err(err)
}

#[tauri::command(async)]
#[allow(clippy::type_complexity)]
pub fn rule_history(state: State<DaemonState>) -> Result<Vec<(u64, String, String, String, bool, String)>, String> {
    state.0.rule_history().map_err(err)
}

#[tauri::command(async)]
pub fn set_rule_enabled(state: State<DaemonState>, rule_id: u32, enabled: bool) -> Result<(), String> {
    state.0.set_rule_enabled(rule_id, enabled).map_err(err)
}

#[tauri::command(async)]
pub fn delete_rule(state: State<DaemonState>, rule_id: u32) -> Result<(), String> {
    state.0.delete_rule(rule_id).map_err(err)
}

// --- Intelligent Audio Ducking ---

#[tauri::command(async)]
#[allow(clippy::type_complexity)]
pub fn list_ducking_rules(state: State<DaemonState>) -> Result<Vec<(u32, bool, u32, Vec<u32>, f32, f32, f32, f32, f32)>, String> {
    state.0.list_ducking_rules().map_err(err)
}

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn create_ducking_rule(
    state: State<DaemonState>,
    trigger_group: u32,
    target_groups: Vec<u32>,
    reduction_db: f32,
    attack_ms: f32,
    hold_ms: f32,
    release_ms: f32,
    threshold_db: f32,
) -> Result<u32, String> {
    state.0.create_ducking_rule(trigger_group, target_groups, reduction_db, attack_ms, hold_ms, release_ms, threshold_db).map_err(err)
}

#[tauri::command(async)]
pub fn set_ducking_rule_enabled(state: State<DaemonState>, rule_id: u32, enabled: bool) -> Result<(), String> {
    state.0.set_ducking_rule_enabled(rule_id, enabled).map_err(err)
}

#[tauri::command(async)]
pub fn delete_ducking_rule(state: State<DaemonState>, rule_id: u32) -> Result<(), String> {
    state.0.delete_ducking_rule(rule_id).map_err(err)
}

// --- MTX Doctor ---

#[tauri::command(async)]
pub fn run_health_checks(state: State<DaemonState>) -> Result<(String, Vec<(String, String, String)>), String> {
    state.0.run_health_checks().map_err(err)
}

// --- Visual Signal-Flow Graph ---

#[tauri::command(async)]
#[allow(clippy::type_complexity)]
pub fn get_graph(state: State<DaemonState>) -> Result<(Vec<(String, String, String, String)>, Vec<(String, String)>), String> {
    state.0.get_graph().map_err(err)
}

// --- Recording ---

#[tauri::command(async)]
pub fn start_recording(state: State<DaemonState>, group_id: u32, path: String, format: String) -> Result<u32, String> {
    state.0.start_recording(group_id, &path, &format).map_err(err)
}

#[tauri::command(async)]
pub fn stop_recording(state: State<DaemonState>, id: u32) -> Result<(), String> {
    state.0.stop_recording(id).map_err(err)
}

#[tauri::command(async)]
#[allow(clippy::type_complexity)]
pub fn list_recordings(state: State<DaemonState>) -> Result<Vec<(u32, u32, String, f32, u64)>, String> {
    state.0.list_recordings().map_err(err)
}

// --- Plugin / DSP architecture ---

#[tauri::command(async)]
pub fn list_available_plugins(state: State<DaemonState>) -> Result<Vec<(String, String, Vec<String>)>, String> {
    state.0.list_available_plugins().map_err(err)
}

#[tauri::command(async)]
#[allow(clippy::type_complexity)]
pub fn list_group_plugins(state: State<DaemonState>, group_id: u32) -> Result<Vec<(u32, String, bool, Vec<String>, Vec<f32>)>, String> {
    state.0.list_group_plugins(group_id).map_err(err)
}

#[tauri::command(async)]
pub fn add_group_plugin(state: State<DaemonState>, group_id: u32, plugin_type: String) -> Result<(), String> {
    state.0.add_group_plugin(group_id, &plugin_type).map_err(err)
}

#[tauri::command(async)]
pub fn remove_group_plugin(state: State<DaemonState>, group_id: u32, index: u32) -> Result<(), String> {
    state.0.remove_group_plugin(group_id, index).map_err(err)
}

#[tauri::command(async)]
pub fn set_group_plugin_bypass(state: State<DaemonState>, group_id: u32, index: u32, bypassed: bool) -> Result<(), String> {
    state.0.set_group_plugin_bypass(group_id, index, bypassed).map_err(err)
}

#[tauri::command(async)]
pub fn set_group_plugin_param(state: State<DaemonState>, group_id: u32, index: u32, param_name: String, value: f32) -> Result<(), String> {
    state.0.set_group_plugin_param(group_id, index, &param_name, value).map_err(err)
}

#[tauri::command(async)]
pub fn reorder_group_plugin(state: State<DaemonState>, group_id: u32, from_index: u32, to_index: u32) -> Result<(), String> {
    state.0.reorder_group_plugin(group_id, from_index, to_index).map_err(err)
}
