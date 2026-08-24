//! Thin D-Bus client for `org.mtxpanel.Daemon` (PROJECT_SPEC.md Section 8).
//! No DSP or routing logic belongs here — every call is a straight
//! pass-through to the daemon, same rule as the Quickshell widget.

use std::sync::Mutex;

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
    fn list_streams(&self) -> zbus::Result<Vec<(u32, String, String, bool)>>;
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
}

/// Tauri-managed state: one shared blocking proxy, connected once at
/// startup and reused by every command.
pub struct DaemonState(pub Mutex<DaemonProxyBlocking<'static>>);

impl DaemonState {
    pub fn connect() -> zbus::Result<Self> {
        let conn = zbus::blocking::Connection::session()?;
        let proxy = DaemonProxyBlocking::new(&conn)?;
        Ok(Self(Mutex::new(proxy)))
    }
}
