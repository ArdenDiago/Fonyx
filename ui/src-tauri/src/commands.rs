//! Tauri commands: each is a one-line forward to the daemon over D-Bus
//! (`daemon.rs`). No state or logic lives here beyond the connection.

use tauri::State;

use crate::daemon::DaemonState;

fn err(e: zbus::Error) -> String {
    e.to_string()
}

#[tauri::command]
pub fn list_groups(state: State<DaemonState>) -> Result<Vec<(u32, String, f32)>, String> {
    state.0.lock().unwrap().list_groups().map_err(err)
}

#[tauri::command]
pub fn list_streams(state: State<DaemonState>) -> Result<Vec<(u32, String)>, String> {
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
pub fn set_gain(state: State<DaemonState>, group_id: u32, value: f32) -> Result<(), String> {
    state.0.lock().unwrap().set_gain(group_id, value).map_err(err)
}

#[tauri::command]
pub fn set_master_gain(state: State<DaemonState>, value: f32) -> Result<(), String> {
    state.0.lock().unwrap().set_master_gain(value).map_err(err)
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
