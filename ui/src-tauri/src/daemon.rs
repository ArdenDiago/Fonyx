//! The UI's handle on the daemon. The D-Bus proxy itself lives in the
//! `fonyx-client` crate, shared with the `fonyx` CLI.

use fonyx_client::DaemonProxyBlocking;

/// Tauri-managed state: one shared blocking proxy, connected once at
/// startup and reused by every command. Proxies are Send + Sync, so no
/// Mutex: one stuck call must not make every other command wait behind it.
pub struct DaemonState(pub DaemonProxyBlocking<'static>);

impl DaemonState {
    pub fn connect() -> zbus::Result<Self> {
        fonyx_client::connect().map(Self)
    }
}
