mod config;
mod dbus;
mod dsp;
mod group;
mod pw;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn main() {
    tracing_subscriber::fmt::init();

    let cfg = config::Config::load_or_default();
    tracing::info!(path = %config::Config::path().display(), "loaded config");

    let state = Arc::new(Mutex::new(cfg));
    let streams = Arc::new(Mutex::new(HashMap::new()));
    let (cmd_tx, cmd_rx) = pipewire::channel::channel::<pw::Command>();

    {
        let state = state.clone();
        let streams = streams.clone();
        std::thread::spawn(move || dbus::serve(state, streams, cmd_tx));
    }

    let initial = state.lock().unwrap().clone();

    if let Err(err) = pw::run(initial, cmd_rx, streams) {
        tracing::error!(%err, "pipewire loop exited with error");
        std::process::exit(1);
    }
}
