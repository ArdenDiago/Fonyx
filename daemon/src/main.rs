mod config;
mod dbus;
mod dsp;
mod group;
mod pw;

fn main() {
    tracing_subscriber::fmt::init();

    let cfg = config::Config::load_or_default();
    tracing::info!(path = %config::Config::path().display(), "loaded config");

    // Stage 1 (see PROJECT_SPEC.md Section 9): connect to PipeWire and log
    // every per-app audio stream we see. No routing, no DSP yet.
    if let Err(err) = pw::run(cfg) {
        tracing::error!(%err, "pipewire loop exited with error");
        std::process::exit(1);
    }
}
