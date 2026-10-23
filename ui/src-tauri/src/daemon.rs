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

/// Snap only (`$SNAP` set): snapd's user daemons are still experimental, so
/// the snap has no service. Opening the app starts the daemon if nothing
/// owns its bus name, and drops the autostart entry that the snap's
/// `autostart:` key turns into "start fonyx.daemon at login".
/// ponytail: no restart on crash in the snap; a user service with a watchdog
/// once snapd's user-daemons feature is stable.
pub fn start_snap_daemon() {
    let Some(snap) = std::env::var_os("SNAP") else { return };
    let running = zbus::blocking::Connection::session()
        .and_then(|c| zbus::blocking::fdo::DBusProxy::new(&c)?.name_has_owner("org.fonyx.Daemon".try_into()?).map_err(Into::into))
        .unwrap_or(true); // no bus: starting a daemon would not help
    if !running {
        use std::os::unix::process::CommandExt;
        let _ = std::process::Command::new(std::path::Path::new(&snap).join("bin/fonyx-daemon"))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0) // outlives the window
            .spawn();
    }
    // XDG_CONFIG_HOME is $SNAP_USER_DATA/.config here, where snapd looks.
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").map(|d| std::path::PathBuf::from(d).join("autostart")) {
        let file = dir.join("fonyx-daemon.desktop");
        if !file.exists() {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(file, "[Desktop Entry]\nType=Application\nName=Fonyx daemon\nExec=/snap/bin/fonyx.daemon\n");
        }
    }
}
