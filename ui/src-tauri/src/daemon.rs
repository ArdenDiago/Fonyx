//! Thin D-Bus client for `org.mtxpanel.Daemon` (PROJECT_SPEC.md Section 8).
//! No DSP or routing logic belongs here — every call is a straight
//! pass-through to the daemon, same rule as the Quickshell widget.

#[zbus::proxy(
    interface = "org.mtxpanel.Daemon",
    default_service = "org.mtxpanel.Daemon",
    default_path = "/org/mtxpanel/Daemon"
)]
pub trait Daemon {
    fn create_group(&self, name: &str) -> zbus::Result<u32>;
    fn delete_group(&self, group_id: u32) -> zbus::Result<()>;
    fn route_stream_to_group(&self, stream_id: u32, group_id: u32) -> zbus::Result<()>;
    fn unroute_stream(&self, stream_id: u32) -> zbus::Result<()>;
    fn set_gain(&self, group_id: u32, value: f32) -> zbus::Result<()>;
    fn set_master_gain(&self, value: f32) -> zbus::Result<()>;
    fn get_master_gain(&self) -> zbus::Result<f32>;
    #[allow(clippy::too_many_arguments)]
    fn set_eq_band(&self, group_id: u32, band_index: u32, freq: f32, gain: f32, q: f32, enabled: bool) -> zbus::Result<()>;
    fn set_highpass(&self, group_id: u32, freq: f32, q: f32, enabled: bool) -> zbus::Result<()>;
    fn set_lowpass(&self, group_id: u32, freq: f32, q: f32, enabled: bool) -> zbus::Result<()>;
    #[allow(clippy::too_many_arguments)]
    fn set_compressor(
        &self,
        group_id: u32,
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        makeup_gain_db: f32,
        enabled: bool,
    ) -> zbus::Result<()>;
    fn set_gate(&self, group_id: u32, threshold_db: f32, attack_ms: f32, release_ms: f32, hold_ms: f32, enabled: bool) -> zbus::Result<()>;
    fn list_groups(&self) -> zbus::Result<Vec<(u32, String, f32, Vec<u32>, bool)>>;
    fn list_streams(&self) -> zbus::Result<Vec<(u32, String, String, bool, String)>>;
    fn get_stream_levels(&self) -> zbus::Result<Vec<(u32, f32)>>;
    #[allow(clippy::type_complexity)]
    fn get_group_details(
        &self,
        group_id: u32,
    ) -> zbus::Result<(
        Vec<(f32, f32, f32, bool)>,
        (f32, f32, bool),
        (f32, f32, bool),
        (f32, f32, f32, f32, f32, bool),
        (f32, f32, f32, f32, bool),
    )>;
    fn get_levels(&self) -> zbus::Result<Vec<(u32, f32, f32, f32, f32)>>;
    fn get_spectrum(&self, group_id: u32) -> zbus::Result<Vec<f32>>;
    fn get_metrics(&self) -> zbus::Result<(f32, f32, u32, u32)>;
    fn set_stream_muted(&self, stream_id: u32, muted: bool) -> zbus::Result<bool>;

    // --- Multi-output routing & delay compensation ---
    #[allow(clippy::type_complexity)]
    fn list_outputs(&self) -> zbus::Result<Vec<(String, String, String, String, bool, bool, u32, u32)>>;
    fn get_output_volume(&self, output_id: &str) -> zbus::Result<(f32, bool)>;
    fn set_output_volume(&self, output_id: &str, volume: f32) -> zbus::Result<bool>;
    fn set_output_muted(&self, output_id: &str, muted: bool) -> zbus::Result<bool>;
    #[allow(clippy::type_complexity)]
    fn list_group_output_routes(&self, group_id: u32) -> zbus::Result<Vec<(String, f32, bool, String, String, f32, bool)>>;
    fn set_group_output_route(&self, group_id: u32, output_id: &str, gain: f32, enabled: bool) -> zbus::Result<()>;
    fn remove_group_output_route(&self, group_id: u32, output_id: &str) -> zbus::Result<()>;
    fn set_output_synchronization(&self, group_id: u32, output_id: &str, sync_mode: &str, manual_delay_ms: f32) -> zbus::Result<()>;
    #[allow(clippy::type_complexity)]
    fn list_master_output_routes(&self) -> zbus::Result<Vec<(String, f32, bool, String, String, f32, bool)>>;
    fn set_master_output_route(&self, output_id: &str, gain: f32, enabled: bool) -> zbus::Result<()>;
    fn remove_master_output_route(&self, output_id: &str) -> zbus::Result<()>;
    fn set_master_output_synchronization(&self, output_id: &str, sync_mode: &str, manual_delay_ms: f32) -> zbus::Result<()>;
    fn set_max_compensation_ms(&self, value: f32) -> zbus::Result<()>;
    fn get_max_compensation_ms(&self) -> zbus::Result<f32>;
    fn set_cue_output(&self, output_id: &str, enabled: bool) -> zbus::Result<()>;
    fn list_cue_outputs(&self) -> zbus::Result<Vec<String>>;
    fn get_cue_group_id(&self) -> zbus::Result<u32>;
    fn set_cue_duck_level(&self, value: f32) -> zbus::Result<()>;
    fn get_cue_duck_level(&self) -> zbus::Result<f32>;

    // --- Settings tab ---
    fn set_preferred_default_output(&self, output_id: &str) -> zbus::Result<()>;
    fn get_preferred_default_output(&self) -> zbus::Result<String>;
    fn set_buffer_ms(&self, value: f32) -> zbus::Result<()>;
    fn get_buffer_ms(&self) -> zbus::Result<f32>;

    // --- Audio Scenes ---
    fn list_scenes(&self) -> zbus::Result<Vec<(u32, String, String)>>;
    fn capture_current_as_scene(&self, name: &str) -> zbus::Result<u32>;
    fn update_scene(&self, scene_id: u32) -> zbus::Result<bool>;
    fn duplicate_scene(&self, scene_id: u32, new_name: &str) -> zbus::Result<u32>;
    fn delete_scene(&self, scene_id: u32) -> zbus::Result<()>;
    fn activate_scene(&self, scene_id: u32) -> zbus::Result<()>;

    // --- Per-App Audio Profiles ---
    fn save_profile_for_stream(&self, stream_id: u32, label: &str) -> zbus::Result<bool>;

    fn list_group_feeds(&self) -> zbus::Result<Vec<(u32, u32)>>;
    fn create_app_strip(&self, stream_id: u32) -> zbus::Result<u32>;

    // --- Rules engine ---
    #[allow(clippy::type_complexity)]
    fn list_rules(&self) -> zbus::Result<Vec<(u32, String, bool, i32, u64, String, String)>>;
    fn get_rule(&self, rule_id: u32) -> zbus::Result<String>;
    fn save_rule(&self, json: &str) -> zbus::Result<u32>;
    fn rule_history(&self) -> zbus::Result<Vec<(u64, String, String, String, bool, String)>>;
    fn set_rule_enabled(&self, rule_id: u32, enabled: bool) -> zbus::Result<()>;
    fn delete_rule(&self, rule_id: u32) -> zbus::Result<()>;

    // --- Intelligent Audio Ducking ---
    #[allow(clippy::type_complexity)]
    fn list_ducking_rules(&self) -> zbus::Result<Vec<(u32, bool, u32, Vec<u32>, f32, f32, f32, f32, f32)>>;
    #[allow(clippy::too_many_arguments)]
    fn create_ducking_rule(&self, trigger_group: u32, target_groups: Vec<u32>, reduction_db: f32, attack_ms: f32, hold_ms: f32, release_ms: f32, threshold_db: f32) -> zbus::Result<u32>;
    fn set_ducking_rule_enabled(&self, rule_id: u32, enabled: bool) -> zbus::Result<()>;
    fn delete_ducking_rule(&self, rule_id: u32) -> zbus::Result<()>;

    // --- MTX Doctor ---
    fn run_health_checks(&self) -> zbus::Result<(String, Vec<(String, String, String)>)>;

    // --- Visual Signal-Flow Graph ---
    #[allow(clippy::type_complexity)]
    fn get_graph(&self) -> zbus::Result<(Vec<(String, String, String, String)>, Vec<(String, String)>)>;

    // --- Recording ---
    fn start_recording(&self, group_id: u32, path: &str, format: &str) -> zbus::Result<u32>;
    fn stop_recording(&self, id: u32) -> zbus::Result<()>;
    #[allow(clippy::type_complexity)]
    fn list_recordings(&self) -> zbus::Result<Vec<(u32, u32, String, f32, u64)>>;

    // --- Plugin / DSP architecture ---
    fn list_available_plugins(&self) -> zbus::Result<Vec<(String, String, Vec<(String, f32, f32, f32)>)>>;
    #[allow(clippy::type_complexity)]
    fn list_group_plugins(&self, group_id: u32) -> zbus::Result<Vec<(u32, String, bool, Vec<String>, Vec<f32>)>>;
    fn add_group_plugin(&self, group_id: u32, plugin_type: &str) -> zbus::Result<()>;
    fn remove_group_plugin(&self, group_id: u32, index: u32) -> zbus::Result<()>;
    fn set_group_plugin_bypass(&self, group_id: u32, index: u32, bypassed: bool) -> zbus::Result<()>;
    fn set_group_plugin_param(&self, group_id: u32, index: u32, param_name: &str, value: f32) -> zbus::Result<()>;
    fn reorder_group_plugin(&self, group_id: u32, from_index: u32, to_index: u32) -> zbus::Result<()>;
}

/// Tauri-managed state: one shared blocking proxy, connected once at
/// startup and reused by every command. Proxies are Send + Sync, so no
/// Mutex: one stuck call must not make every other command wait behind it.
pub struct DaemonState(pub DaemonProxyBlocking<'static>);

impl DaemonState {
    pub fn connect() -> zbus::Result<Self> {
        // Bounds every call: a wedged daemon makes that one call fail after
        // 3s instead of holding a thread-pool worker forever.
        let conn = zbus::blocking::connection::Builder::session()?
            .method_timeout(std::time::Duration::from_secs(3))
            .build()?;
        let proxy = DaemonProxyBlocking::new(&conn)?;
        Ok(Self(proxy))
    }
}
