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
use crate::pw::{AppStream, Command};

pub struct Daemon {
    state: Arc<Mutex<Config>>,
    streams: Arc<Mutex<HashMap<u32, AppStream>>>,
    cmd_tx: pipewire::channel::Sender<Command>,
}

impl Daemon {
    pub fn new(
        state: Arc<Mutex<Config>>,
        streams: Arc<Mutex<HashMap<u32, AppStream>>>,
        cmd_tx: pipewire::channel::Sender<Command>,
    ) -> Self {
        Self { state, streams, cmd_tx }
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
        cfg.groups.retain(|g| g.id != group_id);
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::DeleteGroup(group_id));
    }

    fn route_stream_to_group(&mut self, stream_id: u32, group_id: u32) {
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            if !group.routed_streams.contains(&stream_id) {
                group.routed_streams.push(stream_id);
            }
        }
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::RouteStream { stream_id, group_id });
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

    /// (id, name, gain) per group.
    fn list_groups(&self) -> Vec<(u32, String, f32)> {
        let cfg = self.state.lock().unwrap();
        cfg.groups.iter().map(|g| (g.id, g.name.clone(), g.gain)).collect()
    }

    /// (id, name) per detected app stream.
    fn list_streams(&self) -> Vec<(u32, String)> {
        let streams = self.streams.lock().unwrap();
        streams.values().map(|s| (s.id, s.name.clone())).collect()
    }
}

/// Blocks the calling thread forever serving the D-Bus interface. Meant to
/// be run on its own `std::thread`.
pub fn serve(
    state: Arc<Mutex<Config>>,
    streams: Arc<Mutex<HashMap<u32, AppStream>>>,
    cmd_tx: pipewire::channel::Sender<Command>,
) {
    let daemon = Daemon::new(state, streams, cmd_tx);
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
