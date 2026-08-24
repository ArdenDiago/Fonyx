mod config;
mod dbus;
mod dsp;
mod group;
mod pw;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn main() {
    tracing_subscriber::fmt::init();

    let mut cfg = config::Config::load_or_default();
    tracing::info!(path = %config::Config::path().display(), "loaded config");
    if cfg.ensure_default_group() {
        tracing::info!("created default catch-all group (first run)");
        let _ = cfg.save();
    }

    let state = Arc::new(Mutex::new(cfg));
    let streams = Arc::new(Mutex::new(HashMap::new()));
    let live = Arc::new(Mutex::new(HashMap::new()));
    let metrics = Arc::new(pw::Metrics::default());
    let (cmd_tx, cmd_rx) = pipewire::channel::channel::<pw::Command>();

    {
        let state = state.clone();
        let streams = streams.clone();
        let live = live.clone();
        let metrics = metrics.clone();
        std::thread::spawn(move || dbus::serve(state, streams, cmd_tx, live, metrics));
    }
    {
        let metrics = metrics.clone();
        std::thread::spawn(move || metrics_loop(metrics));
    }

    let initial = state.lock().unwrap().clone();

    if let Err(err) = pw::run(initial, cmd_rx, streams, live, metrics, state.clone()) {
        tracing::error!(%err, "pipewire loop exited with error");
        std::process::exit(1);
    }
}

/// Refreshes system-wide CPU/RAM once a second for the Home tab (Section
/// 7.1) — cheap enough on its own thread, and keeps `sysinfo`'s blocking
/// refresh call far away from the audio callback.
fn metrics_loop(metrics: Arc<pw::Metrics>) {
    let mut sys = sysinfo::System::new();
    loop {
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        metrics.set_cpu_pct(sys.global_cpu_usage());
        let ram_pct = if sys.total_memory() > 0 {
            sys.used_memory() as f32 / sys.total_memory() as f32 * 100.0
        } else {
            0.0
        };
        metrics.set_ram_pct(ram_pct);
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
