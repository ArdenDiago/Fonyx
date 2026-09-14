//! Tauri commands: each is a one-line forward to the daemon over D-Bus
//! (`daemon.rs`). No state or logic lives here beyond the connection.

use tauri::State;

use crate::daemon::DaemonState;

fn err(e: zbus::Error) -> String {
    e.to_string()
}

#[tauri::command]
pub fn list_groups(state: State<DaemonState>) -> Result<Vec<(u32, String, f32, Vec<u32>, bool)>, String> {
    state.0.lock().unwrap().list_groups().map_err(err)
}

#[tauri::command]
pub fn list_streams(state: State<DaemonState>) -> Result<Vec<(u32, String, String, bool, String)>, String> {
    state.0.lock().unwrap().list_streams().map_err(err)
}

#[tauri::command]
pub fn create_group(state: State<DaemonState>, name: String) -> Result<u32, String> {
    state.0.lock().unwrap().create_group(&name).map_err(err)
}

#[tauri::command]
pub fn delete_group(state: State<DaemonState>, group_id: u32) -> Result<(), String> {
    state.0.lock().unwrap().delete_group(group_id).map_err(err)
}

#[tauri::command]
pub fn route_stream_to_group(state: State<DaemonState>, stream_id: u32, group_id: u32) -> Result<(), String> {
    state.0.lock().unwrap().route_stream_to_group(stream_id, group_id).map_err(err)
}

#[tauri::command]
pub fn unroute_stream(state: State<DaemonState>, stream_id: u32) -> Result<(), String> {
    state.0.lock().unwrap().unroute_stream(stream_id).map_err(err)
}

#[tauri::command]
pub fn set_gain(state: State<DaemonState>, group_id: u32, value: f32) -> Result<(), String> {
    state.0.lock().unwrap().set_gain(group_id, value).map_err(err)
}

#[tauri::command]
pub fn set_master_gain(state: State<DaemonState>, value: f32) -> Result<(), String> {
    state.0.lock().unwrap().set_master_gain(value).map_err(err)
}

#[tauri::command]
pub fn get_master_gain(state: State<DaemonState>) -> Result<f32, String> {
    state.0.lock().unwrap().get_master_gain().map_err(err)
}

#[tauri::command]
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
    state.0.lock().unwrap().set_eq_band(group_id, band_index, freq, gain, q, enabled).map_err(err)
}

#[tauri::command]
pub fn set_highpass(state: State<DaemonState>, group_id: u32, freq: f32, q: f32, enabled: bool) -> Result<(), String> {
    state.0.lock().unwrap().set_highpass(group_id, freq, q, enabled).map_err(err)
}

#[tauri::command]
pub fn set_lowpass(state: State<DaemonState>, group_id: u32, freq: f32, q: f32, enabled: bool) -> Result<(), String> {
    state.0.lock().unwrap().set_lowpass(group_id, freq, q, enabled).map_err(err)
}

#[tauri::command]
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
        .lock()
        .unwrap()
        .set_compressor(group_id, threshold_db, ratio, attack_ms, release_ms, makeup_gain_db, enabled)
        .map_err(err)
}

#[tauri::command]
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
    state.0.lock().unwrap().get_group_details(group_id).map_err(err)
}

#[tauri::command]
pub fn get_levels(state: State<DaemonState>) -> Result<Vec<(u32, f32, f32, f32, f32)>, String> {
    state.0.lock().unwrap().get_levels().map_err(err)
}

#[tauri::command]
pub fn get_spectrum(state: State<DaemonState>, group_id: u32) -> Result<Vec<f32>, String> {
    state.0.lock().unwrap().get_spectrum(group_id).map_err(err)
}

#[tauri::command]
pub fn get_metrics(state: State<DaemonState>) -> Result<(f32, f32, u32, u32), String> {
    state.0.lock().unwrap().get_metrics().map_err(err)
}

#[tauri::command]
pub fn set_stream_muted(state: State<DaemonState>, stream_id: u32, muted: bool) -> Result<bool, String> {
    state.0.lock().unwrap().set_stream_muted(stream_id, muted).map_err(err)
}

#[tauri::command]
#[allow(clippy::type_complexity)]
pub fn list_outputs(state: State<DaemonState>) -> Result<Vec<(String, String, String, String, bool, bool, u32, u32)>, String> {
    state.0.lock().unwrap().list_outputs().map_err(err)
}

#[tauri::command]
pub fn get_output_latency(state: State<DaemonState>, output_id: String) -> Result<(bool, f32), String> {
    state.0.lock().unwrap().get_output_latency(&output_id).map_err(err)
}

#[tauri::command]
#[allow(clippy::type_complexity)]
pub fn list_group_output_routes(state: State<DaemonState>, group_id: u32) -> Result<Vec<(String, f32, bool, String, String, f32, bool)>, String> {
    state.0.lock().unwrap().list_group_output_routes(group_id).map_err(err)
}

#[tauri::command]
pub fn set_group_output_route(state: State<DaemonState>, group_id: u32, output_id: String, gain: f32, enabled: bool) -> Result<(), String> {
    state.0.lock().unwrap().set_group_output_route(group_id, &output_id, gain, enabled).map_err(err)
}

#[tauri::command]
pub fn remove_group_output_route(state: State<DaemonState>, group_id: u32, output_id: String) -> Result<(), String> {
    state.0.lock().unwrap().remove_group_output_route(group_id, &output_id).map_err(err)
}

#[tauri::command]
pub fn set_output_synchronization(
    state: State<DaemonState>,
    group_id: u32,
    output_id: String,
    sync_mode: String,
    manual_delay_ms: f32,
) -> Result<(), String> {
    state.0.lock().unwrap().set_output_synchronization(group_id, &output_id, &sync_mode, manual_delay_ms).map_err(err)
}

#[tauri::command]
pub fn set_max_compensation_ms(state: State<DaemonState>, value: f32) -> Result<(), String> {
    state.0.lock().unwrap().set_max_compensation_ms(value).map_err(err)
}

#[tauri::command]
pub fn get_max_compensation_ms(state: State<DaemonState>) -> Result<f32, String> {
    state.0.lock().unwrap().get_max_compensation_ms().map_err(err)
}

#[tauri::command]
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
    state.0.lock().unwrap().set_gate(group_id, threshold_db, attack_ms, release_ms, hold_ms, enabled).map_err(err)
}

// --- Audio Scenes ---

#[tauri::command]
pub fn list_scenes(state: State<DaemonState>) -> Result<Vec<(u32, String, String)>, String> {
    state.0.lock().unwrap().list_scenes().map_err(err)
}

#[tauri::command]
pub fn capture_current_as_scene(state: State<DaemonState>, name: String) -> Result<u32, String> {
    state.0.lock().unwrap().capture_current_as_scene(&name).map_err(err)
}

#[tauri::command]
pub fn update_scene(state: State<DaemonState>, scene_id: u32) -> Result<bool, String> {
    state.0.lock().unwrap().update_scene(scene_id).map_err(err)
}

#[tauri::command]
pub fn rename_scene(state: State<DaemonState>, scene_id: u32, new_name: String) -> Result<(), String> {
    state.0.lock().unwrap().rename_scene(scene_id, &new_name).map_err(err)
}

#[tauri::command]
pub fn duplicate_scene(state: State<DaemonState>, scene_id: u32, new_name: String) -> Result<u32, String> {
    state.0.lock().unwrap().duplicate_scene(scene_id, &new_name).map_err(err)
}

#[tauri::command]
pub fn delete_scene(state: State<DaemonState>, scene_id: u32) -> Result<(), String> {
    state.0.lock().unwrap().delete_scene(scene_id).map_err(err)
}

#[tauri::command]
pub fn activate_scene(state: State<DaemonState>, scene_id: u32) -> Result<(u32, u32, u32, u32), String> {
    state.0.lock().unwrap().activate_scene(scene_id).map_err(err)
}

// --- Per-App Audio Profiles ---

#[tauri::command]
#[allow(clippy::type_complexity)]
pub fn list_profiles(state: State<DaemonState>) -> Result<Vec<(String, String, f32, u32, String)>, String> {
    state.0.lock().unwrap().list_profiles().map_err(err)
}

#[tauri::command]
pub fn save_profile_for_stream(state: State<DaemonState>, stream_id: u32, label: String) -> Result<bool, String> {
    state.0.lock().unwrap().save_profile_for_stream(stream_id, &label).map_err(err)
}

#[tauri::command]
pub fn apply_profile_to_app(state: State<DaemonState>, app_id: String) -> Result<bool, String> {
    state.0.lock().unwrap().apply_profile_to_app(&app_id).map_err(err)
}

#[tauri::command]
pub fn delete_profile(state: State<DaemonState>, app_id: String) -> Result<(), String> {
    state.0.lock().unwrap().delete_profile(&app_id).map_err(err)
}

// --- Rules engine ---

#[tauri::command]
#[allow(clippy::type_complexity)]
pub fn list_rules(state: State<DaemonState>) -> Result<Vec<(u32, String, bool, i32, u64, String, String)>, String> {
    state.0.lock().unwrap().list_rules().map_err(err)
}

#[tauri::command]
pub fn create_auto_route_rule(
    state: State<DaemonState>,
    name: String,
    template: String,
    app_id: String,
    service: String,
    group_id: u32,
) -> Result<u32, String> {
    state.0.lock().unwrap().create_auto_route_rule(&name, &template, &app_id, &service, group_id).map_err(err)
}

#[tauri::command]
pub fn create_device_connected_rule(state: State<DaemonState>, name: String, output_id: String, group_id: u32, route_gain: f32) -> Result<u32, String> {
    state.0.lock().unwrap().create_device_connected_rule(&name, &output_id, group_id, route_gain).map_err(err)
}

#[tauri::command]
pub fn set_rule_enabled(state: State<DaemonState>, rule_id: u32, enabled: bool) -> Result<(), String> {
    state.0.lock().unwrap().set_rule_enabled(rule_id, enabled).map_err(err)
}

#[tauri::command]
pub fn delete_rule(state: State<DaemonState>, rule_id: u32) -> Result<(), String> {
    state.0.lock().unwrap().delete_rule(rule_id).map_err(err)
}

// --- Intelligent Audio Ducking ---

#[tauri::command]
#[allow(clippy::type_complexity)]
pub fn list_ducking_rules(state: State<DaemonState>) -> Result<Vec<(u32, bool, u32, Vec<u32>, f32, f32, f32, f32, f32)>, String> {
    state.0.lock().unwrap().list_ducking_rules().map_err(err)
}

#[tauri::command]
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
    state.0.lock().unwrap().create_ducking_rule(trigger_group, target_groups, reduction_db, attack_ms, hold_ms, release_ms, threshold_db).map_err(err)
}

#[tauri::command]
pub fn set_ducking_rule_enabled(state: State<DaemonState>, rule_id: u32, enabled: bool) -> Result<(), String> {
    state.0.lock().unwrap().set_ducking_rule_enabled(rule_id, enabled).map_err(err)
}

#[tauri::command]
pub fn delete_ducking_rule(state: State<DaemonState>, rule_id: u32) -> Result<(), String> {
    state.0.lock().unwrap().delete_ducking_rule(rule_id).map_err(err)
}

// --- MTX Doctor ---

#[tauri::command]
pub fn run_health_checks(state: State<DaemonState>) -> Result<(String, Vec<(String, String, String)>), String> {
    state.0.lock().unwrap().run_health_checks().map_err(err)
}

// --- Visual Signal-Flow Graph ---

#[tauri::command]
#[allow(clippy::type_complexity)]
pub fn get_graph(state: State<DaemonState>) -> Result<(Vec<(String, String, String, String)>, Vec<(String, String)>), String> {
    state.0.lock().unwrap().get_graph().map_err(err)
}

// --- Recording ---

#[tauri::command]
pub fn start_recording(state: State<DaemonState>, group_id: u32, path: String, format: String) -> Result<u32, String> {
    state.0.lock().unwrap().start_recording(group_id, &path, &format).map_err(err)
}

#[tauri::command]
pub fn stop_recording(state: State<DaemonState>, id: u32) -> Result<(), String> {
    state.0.lock().unwrap().stop_recording(id).map_err(err)
}

#[tauri::command]
#[allow(clippy::type_complexity)]
pub fn list_recordings(state: State<DaemonState>) -> Result<Vec<(u32, u32, String, f32, u64)>, String> {
    state.0.lock().unwrap().list_recordings().map_err(err)
}

// --- Plugin / DSP architecture ---

#[tauri::command]
pub fn list_available_plugins(state: State<DaemonState>) -> Result<Vec<(String, String, Vec<String>)>, String> {
    state.0.lock().unwrap().list_available_plugins().map_err(err)
}

#[tauri::command]
#[allow(clippy::type_complexity)]
pub fn list_group_plugins(state: State<DaemonState>, group_id: u32) -> Result<Vec<(u32, String, bool, Vec<String>, Vec<f32>)>, String> {
    state.0.lock().unwrap().list_group_plugins(group_id).map_err(err)
}

#[tauri::command]
pub fn add_group_plugin(state: State<DaemonState>, group_id: u32, plugin_type: String) -> Result<(), String> {
    state.0.lock().unwrap().add_group_plugin(group_id, &plugin_type).map_err(err)
}

#[tauri::command]
pub fn remove_group_plugin(state: State<DaemonState>, group_id: u32, index: u32) -> Result<(), String> {
    state.0.lock().unwrap().remove_group_plugin(group_id, index).map_err(err)
}

#[tauri::command]
pub fn set_group_plugin_bypass(state: State<DaemonState>, group_id: u32, index: u32, bypassed: bool) -> Result<(), String> {
    state.0.lock().unwrap().set_group_plugin_bypass(group_id, index, bypassed).map_err(err)
}

#[tauri::command]
pub fn set_group_plugin_param(state: State<DaemonState>, group_id: u32, index: u32, param_name: String, value: f32) -> Result<(), String> {
    state.0.lock().unwrap().set_group_plugin_param(group_id, index, &param_name, value).map_err(err)
}

#[tauri::command]
pub fn reorder_group_plugin(state: State<DaemonState>, group_id: u32, from_index: u32, to_index: u32) -> Result<(), String> {
    state.0.lock().unwrap().reorder_group_plugin(group_id, from_index, to_index).map_err(err)
}
