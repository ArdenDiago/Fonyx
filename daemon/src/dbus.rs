//! D-Bus interface (PROJECT_SPEC.md Section 8, build-order stage 4):
//! `org.mtxpanel.Daemon` on the session bus. Every method updates the
//! persisted `Config` (so state survives a daemon restart) and forwards a
//! `Command` to the PipeWire engine thread so the change takes effect live.
//!
//! Runs on `zbus::blocking`, which manages its own connection thread
//! internally — no extra async runtime dependency needed just to host a
//! handful of synchronous methods.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zbus::interface;

use crate::config::Config;
use crate::dsp::biquad::{EqBand, FilterSlope};
use crate::dsp::dynamics::{CompressorParams, GateParams};
use crate::group::Group;
use crate::pw::{AppStream, Command, GroupLive, Metrics};

pub struct Daemon {
    state: Arc<Mutex<Config>>,
    streams: Arc<Mutex<HashMap<u32, AppStream>>>,
    cmd_tx: pipewire::channel::Sender<Command>,
    live: Arc<Mutex<HashMap<u32, Arc<GroupLive>>>>,
    metrics: Arc<Metrics>,
}

impl Daemon {
    pub fn new(
        state: Arc<Mutex<Config>>,
        streams: Arc<Mutex<HashMap<u32, AppStream>>>,
        cmd_tx: pipewire::channel::Sender<Command>,
        live: Arc<Mutex<HashMap<u32, Arc<GroupLive>>>>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self { state, streams, cmd_tx, live, metrics }
    }

    fn send(&self, cmd: Command) {
        if self.cmd_tx.send(cmd).is_err() {
            tracing::error!("pipewire engine channel closed, command dropped");
        }
    }
}

#[interface(name = "org.mtxpanel.Daemon")]
impl Daemon {
    fn create_group(&mut self, name: String) -> u32 {
        let mut cfg = self.state.lock().unwrap();
        let id = cfg.groups.iter().map(|g| g.id).max().map_or(0, |max| max + 1);
        let group = Group::new(id, name);
        cfg.groups.push(group.clone());
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::CreateGroup(group));
        id
    }

    fn delete_group(&mut self, group_id: u32) {
        let mut cfg = self.state.lock().unwrap();
        // The protected default group is the app's always-on primary bus —
        // never removable (Section: default live-audio bus). The UI already
        // hides its delete button; this is the server-side guard.
        if cfg.groups.iter().any(|g| g.id == group_id && g.protected) {
            tracing::warn!(group_id, "refused to delete the protected default group");
            return;
        }
        cfg.groups.retain(|g| g.id != group_id);
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::DeleteGroup(group_id));
    }

    fn route_stream_to_group(&mut self, stream_id: u32, group_id: u32) {
        let mut cfg = self.state.lock().unwrap();
        // A stream only ever feeds one group at a time (the engine now
        // severs its old link when re-routing — see pw.rs's route_stream),
        // so drop it from every other group's list first to keep the
        // persisted state exclusive too.
        for group in cfg.groups.iter_mut() {
            group.routed_streams.retain(|&id| id != stream_id);
        }
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            group.routed_streams.push(stream_id);
        }
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::RouteStream { stream_id, group_id });
    }

    /// Takes a stream back out of whatever group it's in and reconnects it
    /// to hardware (Section 7.2's "removed from the group" direction).
    fn unroute_stream(&mut self, stream_id: u32) {
        let mut cfg = self.state.lock().unwrap();
        for group in cfg.groups.iter_mut() {
            group.routed_streams.retain(|&id| id != stream_id);
        }
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::UnrouteStream { stream_id });
    }

    fn set_gain(&mut self, group_id: u32, value: f32) {
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            group.gain = value;
        }
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::SetGain { group_id, value });
    }

    /// Master fader (Section 7.4): one multiplier applied on top of every
    /// group's own gain.
    fn set_master_gain(&mut self, value: f32) {
        let mut cfg = self.state.lock().unwrap();
        cfg.master_gain = value;
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::SetMasterGain(value));
    }

    /// Reads the real persisted master gain — so every UI surface showing
    /// a master fader (Home, Faders) hydrates from the same source of
    /// truth instead of each assuming its own "1.0" default.
    fn get_master_gain(&self) -> f32 {
        self.state.lock().unwrap().master_gain
    }

    fn set_eq_band(&mut self, group_id: u32, band_index: u32, freq: f32, gain: f32, q: f32, enabled: bool) {
        let band = EqBand { freq_hz: freq, gain_db: gain, q, enabled };
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            if let Some(slot) = group.eq_bands.get_mut(band_index as usize) {
                *slot = band;
            }
        }
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::SetEqBand { group_id, index: band_index as usize, band });
    }

    fn set_highpass(&mut self, group_id: u32, freq: f32, q: f32, enabled: bool) {
        let filter = FilterSlope { freq_hz: freq, q, enabled };
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            group.highpass = filter;
        }
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::SetHighpass { group_id, filter });
    }

    fn set_lowpass(&mut self, group_id: u32, freq: f32, q: f32, enabled: bool) {
        let filter = FilterSlope { freq_hz: freq, q, enabled };
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            group.lowpass = filter;
        }
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::SetLowpass { group_id, filter });
    }

    #[allow(clippy::too_many_arguments)]
    fn set_compressor(
        &mut self,
        group_id: u32,
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        makeup_gain_db: f32,
        enabled: bool,
    ) {
        let params = CompressorParams { threshold_db, ratio, attack_ms, release_ms, makeup_gain_db, enabled };
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            group.compressor = params;
        }
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::SetCompressor { group_id, params });
    }

    fn set_gate(&mut self, group_id: u32, threshold_db: f32, attack_ms: f32, release_ms: f32, hold_ms: f32, enabled: bool) {
        let params = GateParams { threshold_db, attack_ms, release_ms, hold_ms, enabled };
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            group.gate = params;
        }
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::SetGate { group_id, params });
    }

    /// (id, name, gain, routed_stream_ids, is_protected) per group — the
    /// routed-ids list is what lets a UI client remove a stream from its
    /// Inputs view once it's actually feeding a group (Section 7.2), instead
    /// of only tracking that client-side for the current session. The
    /// protected flag marks the one always-on default group, whose delete
    /// control the UI hides.
    fn list_groups(&self) -> Vec<(u32, String, f32, Vec<u32>, bool)> {
        let cfg = self.state.lock().unwrap();
        cfg.groups
            .iter()
            .map(|g| (g.id, g.name.clone(), g.gain, g.routed_streams.clone(), g.protected))
            .collect()
    }

    /// (id, name, service, is_actively_playing) per detected app stream.
    /// `service` is the extracted site/service name (e.g. "YouTube" out of
    /// a Firefox tab's full title) — empty when the stream doesn't expose
    /// one, in which case the UI just shows `name`.
    fn list_streams(&self) -> Vec<(u32, String, String, bool)> {
        let streams = self.streams.lock().unwrap();
        streams.values().map(|s| (s.id, s.name.clone(), s.service.clone(), s.running)).collect()
    }

    /// Full DSP state for one group (eq bands, HPF, LPF, compressor, gate) —
    /// read straight off the persisted `Config`, which every setter above
    /// already keeps current. Lets a UI client hydrate its controls to what
    /// the daemon is actually running instead of assuming fresh defaults.
    #[allow(clippy::type_complexity)]
    fn get_group_details(
        &self,
        group_id: u32,
    ) -> (
        Vec<(f32, f32, f32, bool)>,
        (f32, f32, bool),
        (f32, f32, bool),
        (f32, f32, f32, f32, f32, bool),
        (f32, f32, f32, f32, bool),
    ) {
        let cfg = self.state.lock().unwrap();
        let Some(g) = cfg.groups.iter().find(|g| g.id == group_id) else {
            return Default::default();
        };
        (
            g.eq_bands.iter().map(|b| (b.freq_hz, b.gain_db, b.q, b.enabled)).collect(),
            (g.highpass.freq_hz, g.highpass.q, g.highpass.enabled),
            (g.lowpass.freq_hz, g.lowpass.q, g.lowpass.enabled),
            (
                g.compressor.threshold_db,
                g.compressor.ratio,
                g.compressor.attack_ms,
                g.compressor.release_ms,
                g.compressor.makeup_gain_db,
                g.compressor.enabled,
            ),
            (g.gate.threshold_db, g.gate.attack_ms, g.gate.release_ms, g.gate.hold_ms, g.gate.enabled),
        )
    }

    /// (group_id, peak_l, peak_r, compressor_gr_db, gate_gr_db) per live
    /// group, for the Home tab's level meters and the MTX Panel's
    /// compression/gate graphs (Section 7.1/7.3). Read lock-free off the
    /// audio callback's published values, not polled from it.
    fn get_levels(&self) -> Vec<(u32, f32, f32, f32, f32)> {
        let live = self.live.lock().unwrap();
        live.iter()
            .map(|(id, l)| {
                let (level_l, level_r, gr_db, gate_gr_db) = l.read();
                (*id, level_l, level_r, gr_db, gate_gr_db)
            })
            .collect()
    }

    /// 24 log-spaced magnitude-in-dB bins (`dsp::spectrum::N_BINS`, 40Hz-
    /// 18kHz) of the group's *input* signal, for the Home tab's spectrum
    /// graph and the MTX Panel's "what am I EQing" overlay. Empty if
    /// `group_id` isn't live.
    fn get_spectrum(&self, group_id: u32) -> Vec<f32> {
        let live = self.live.lock().unwrap();
        live.get(&group_id).map(|l| l.read_spectrum().to_vec()).unwrap_or_default()
    }

    /// (cpu_pct, ram_pct, buffer_frames, underrun_count) for the Home tab's
    /// system metrics tiles (Section 7.1/8's `MetricsUpdated`).
    fn get_metrics(&self) -> (f32, f32, u32, u32) {
        (
            self.metrics.cpu_pct(),
            self.metrics.ram_pct(),
            self.metrics.buffer_frames.load(std::sync::atomic::Ordering::Relaxed),
            self.metrics.underruns.load(std::sync::atomic::Ordering::Relaxed) as u32,
        )
    }

    /// Mutes/unmutes a raw app stream directly (Inputs tab's solo button,
    /// Section 7.2) — deliberately *not* routed through the PipeWire engine
    /// thread: this has nothing to do with our own group graph, it's the
    /// same knob `wpctl`/`pavucontrol` turn on any stream. Shelling out to
    /// the already-installed, already-trusted `wpctl` avoids hand-building
    /// a raw SPA Props pod (channelVolumes/mute) ourselves just for this.
    fn set_stream_muted(&self, stream_id: u32, muted: bool) -> bool {
        std::process::Command::new("wpctl")
            .args(["set-mute", &stream_id.to_string(), if muted { "1" } else { "0" }])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// Blocks the calling thread forever serving the D-Bus interface. Meant to
/// be run on its own `std::thread`.
pub fn serve(
    state: Arc<Mutex<Config>>,
    streams: Arc<Mutex<HashMap<u32, AppStream>>>,
    cmd_tx: pipewire::channel::Sender<Command>,
    live: Arc<Mutex<HashMap<u32, Arc<GroupLive>>>>,
    metrics: Arc<Metrics>,
) {
    let daemon = Daemon::new(state, streams, cmd_tx, live, metrics);
    let _connection = match zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name("org.mtxpanel.Daemon"))
        .and_then(|b| b.serve_at("/org/mtxpanel/Daemon", daemon))
        .and_then(|b| b.build())
    {
        Ok(conn) => conn,
        Err(err) => {
            tracing::error!(%err, "failed to start D-Bus service");
            return;
        }
    };
    tracing::info!("D-Bus service org.mtxpanel.Daemon registered");
    // The connection's own executor thread(s) keep serving requests as long
    // as `_connection` stays alive; just park this thread forever.
    loop {
        std::thread::park();
    }
}
