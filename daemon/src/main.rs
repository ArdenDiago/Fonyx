mod config;
mod dbus;
mod doctor;
mod dsp;
mod ducking;
mod graph;
mod group;
mod identity;
mod output;
mod plugins;
mod profiles;
mod pw;
mod recording;
mod rules;
mod scenes;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn main() {
    tracing_subscriber::fmt::init();
    block_shutdown_signals();

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
    let outputs = Arc::new(Mutex::new(HashMap::new()));
    let recording_status = Arc::new(Mutex::new(HashMap::new()));
    let (cmd_tx, cmd_rx) = pipewire::channel::channel::<pw::Command>();

    {
        let state = state.clone();
        let streams = streams.clone();
        let live = live.clone();
        let metrics = metrics.clone();
        let outputs = outputs.clone();
        let recording_status = recording_status.clone();
        std::thread::spawn(move || dbus::serve(state, streams, cmd_tx, live, metrics, outputs, recording_status));
    }
    {
        let metrics = metrics.clone();
        std::thread::spawn(move || metrics_loop(metrics));
    }

    let initial = state.lock().unwrap().clone();

    if let Err(err) = pw::run(initial, cmd_rx, streams, live, metrics, state.clone(), outputs, recording_status) {
        tracing::error!(%err, "pipewire loop exited with error");
        std::process::exit(1);
    }
}

/// Blocks SIGTERM/SIGINT on the calling (main) thread *before* any other
/// thread is spawned — every thread created afterward (`std::thread::spawn`
/// inherits the creating thread's signal mask) is therefore unreachable by
/// their default terminate-immediately disposition too. This is what makes
/// `pw.rs::run`'s `add_signal_local` handlers (registered later, on the
/// PipeWire mainloop thread) the *only* thing that ever sees these signals,
/// so a graceful stop reliably reconnects live streams to hardware first
/// (`Engine::shutdown_gracefully`) instead of racing the OS default
/// disposition on whichever thread happens to receive the signal — observed
/// live without this: the process died with app streams left unlinked
/// (silent) before the PipeWire-thread handler ever ran.
fn block_shutdown_signals() {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
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
