//! D-Bus interface (PROJECT_SPEC.md Section 8, build-order stage 4):
//! `org.sonicforge.Daemon` on the session bus. Every method updates the
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
use crate::ducking::DuckingRule;
use crate::dsp::biquad::{EqBand, FilterSlope};
use crate::dsp::dynamics::{CompressorParams, GateParams};
use crate::group::{Group, GroupOutputRoute};
use crate::identity::{AppId, OutputId};
use crate::output::{OutputDevice, RouteMode, SyncMode};
use crate::profiles::AppProfile;
use crate::pw::{AppStream, Command, GroupLive, Metrics};
use crate::rules::{Action, Condition, Rule, Trigger};
use crate::scenes::Scene;

pub struct Daemon {
    state: Arc<Mutex<Config>>,
    streams: Arc<Mutex<HashMap<u32, AppStream>>>,
    cmd_tx: pipewire::channel::Sender<Command>,
    live: Arc<Mutex<HashMap<u32, Arc<GroupLive>>>>,
    metrics: Arc<Metrics>,
    outputs: Arc<Mutex<HashMap<OutputId, OutputDevice>>>,
    recording_status: Arc<Mutex<HashMap<u32, crate::pw::RecordingStatus>>>,
    next_recording_id: std::sync::atomic::AtomicU32,
    /// (underrun count, when read) from the previous `run_health_checks`
    /// call — lets that method compute a recent *rate* instead of judging
    /// severity off a lifetime-cumulative total (see
    /// `doctor::HealthInputs::underrun_rate_per_sec`'s doc comment for why
    /// the lifetime count alone is the wrong signal).
    last_underrun_check: Mutex<(u64, std::time::Instant)>,
    /// Kept only to emit signals (NewImprovenments/README.md's design
    /// principle: multiple UI clients stay in sync via D-Bus, not by
    /// re-polling each other) from inside a method that already runs on
    /// this same connection's own thread — see `serve()`'s doc comment for
    /// why this can't just be a fresh `zbus::blocking::Connection::session()`.
    conn: zbus::blocking::Connection,
}

impl Daemon {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: Arc<Mutex<Config>>,
        streams: Arc<Mutex<HashMap<u32, AppStream>>>,
        cmd_tx: pipewire::channel::Sender<Command>,
        live: Arc<Mutex<HashMap<u32, Arc<GroupLive>>>>,
        metrics: Arc<Metrics>,
        outputs: Arc<Mutex<HashMap<OutputId, OutputDevice>>>,
        recording_status: Arc<Mutex<HashMap<u32, crate::pw::RecordingStatus>>>,
        conn: zbus::blocking::Connection,
    ) -> Self {
        Self {
            state,
            streams,
            cmd_tx,
            live,
            metrics,
            outputs,
            recording_status,
            next_recording_id: std::sync::atomic::AtomicU32::new(0),
            last_underrun_check: Mutex::new((0, std::time::Instant::now())),
            conn,
        }
    }

    fn send(&self, cmd: Command) {
        if self.cmd_tx.send(cmd).is_err() {
            tracing::error!("pipewire engine channel closed, command dropped");
        }
    }

    /// Broadcasts a signal from `/org/sonicforge/Daemon` on `conn` — the same
    /// connection this interface is registered on, so subscribers matching
    /// on path+interface see it regardless of which unique name sent it.
    fn emit<B: serde::Serialize + zbus::zvariant::DynamicType>(&self, signal_name: &str, body: &B) {
        let result = self.conn.emit_signal(None::<()>, "/org/sonicforge/Daemon", "org.sonicforge.Daemon", signal_name, body);
        if let Err(err) = result {
            tracing::warn!(%err, signal_name, "failed to emit signal");
        }
    }

    fn sync_group_routes(&self, group_id: u32) {
        self.send(Command::SyncGroupRoutes { group_id });
    }

    fn sync_master_routes(&self) {
        self.send(Command::SyncMasterRoutes);
    }

    /// The Cue group's routing belongs to `cue_outputs` (set_cue_output),
    /// never to the generic route editors.
    fn refuse_if_cue(&self, group_id: u32) -> bool {
        let is_cue = self.state.lock().unwrap().cue_group_id() == Some(group_id);
        if is_cue {
            tracing::warn!(group_id, "refused to edit the Cue group's routing (use SetCueOutput)");
        }
        is_cue
    }
}

#[interface(name = "org.sonicforge.Daemon")]
impl Daemon {
    fn create_group(&mut self, name: String) -> u32 {
        let mut cfg = self.state.lock().unwrap();
        let id = cfg.groups.iter().map(|g| g.id).max().map_or(0, |max| max + 1);
        let group = Group::new(id, name);
        cfg.groups.push(group.clone());
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::CreateGroup(group));
        self.emit("GroupChanged", &(id,));
        id
    }

    fn delete_group(&mut self, group_id: u32) {
        let mut cfg = self.state.lock().unwrap();
        // The protected default group is the app's always-on primary bus —
        // never removable (Section: default live-audio bus). The UI already
        // hides its delete button; this is the server-side guard.
        if cfg.groups.iter().any(|g| g.id == group_id && (g.protected || g.cue)) {
            tracing::warn!(group_id, "refused to delete a permanent group (Default/Cue)");
            return;
        }
        cfg.groups.retain(|g| g.id != group_id);
        for p in cfg.profiles.iter_mut().filter(|p| p.preferred_group == Some(group_id)) {
            p.preferred_group = None;
        }
        // Sub-groups of a deleted parent go back to normal output routing.
        let orphans: Vec<u32> = cfg.groups.iter_mut().filter(|g| g.feeds_group == Some(group_id)).map(|g| {
            g.feeds_group = None;
            g.id
        }).collect();
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::DeleteGroup(group_id));
        for id in orphans {
            self.send(Command::RebuildGroupRoutes { group_id: id });
        }
        self.emit("GroupChanged", &(group_id,));
    }

    /// Makes `group_id` a sub-group feeding `parent_id` (`Group::feeds_group`),
    /// or back to normal output routing with `parent_id = u32::MAX`.
    fn set_group_feeds(&mut self, group_id: u32, parent_id: u32) -> zbus::fdo::Result<()> {
        let mut cfg = self.state.lock().unwrap();
        let parent = (parent_id != u32::MAX).then_some(parent_id);
        if let Some(p) = parent {
            cfg.check_feed(group_id, p).map_err(zbus::fdo::Error::InvalidArgs)?;
        }
        let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) else {
            return Err(zbus::fdo::Error::InvalidArgs(format!("no group {group_id}")));
        };
        group.feeds_group = parent;
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::RebuildGroupRoutes { group_id });
        self.emit("GroupChanged", &(group_id,));
        Ok(())
    }

    /// (group_id, parent_id) for every sub-group.
    fn list_group_feeds(&self) -> Vec<(u32, u32)> {
        self.state.lock().unwrap().groups.iter().filter_map(|g| g.feeds_group.map(|p| (g.id, p))).collect()
    }

    /// Gives `stream_id`'s app its own DSP: a new group named after the app
    /// that feeds the group the stream is in now, with that app's live
    /// streams in the same group moved onto it (not ones the user put in
    /// other groups: every browser tab shares one app id). The app's profile remembers the new group,
    /// so the app lands there again next time it starts. If the stream is
    /// already on a sub-group, returns that group instead.
    fn create_app_strip(&mut self, stream_id: u32) -> zbus::fdo::Result<u32> {
        let Some((app_id, label)) = self.streams.lock().unwrap().get(&stream_id).map(|s| (s.app_id.clone(), s.name.clone())) else {
            return Err(zbus::fdo::Error::InvalidArgs(format!("no stream {stream_id}")));
        };
        let volume = get_stream_volume(stream_id);
        let mut cfg = self.state.lock().unwrap();
        let current = cfg.groups.iter().find(|g| g.routed_streams.contains(&stream_id));
        if let Some(g) = current.filter(|g| g.feeds_group.is_some()) {
            return Ok(g.id); // an old-style nested strip: already its own
        }
        let parent = match current {
            Some(g) if !g.cue => g.id,
            _ => cfg.groups.iter().find(|g| g.protected).map(|g| g.id).ok_or_else(|| zbus::fdo::Error::Failed("no Default group".into()))?,
        };
        let id = cfg.groups.iter().map(|g| g.id).max().map_or(0, |m| m + 1);
        // Not nested any more: a normal top-level group that *starts as a
        // copy* of the one the app was in (EQ, filters, dynamics, plugins,
        // level, outputs), so it sounds the same until tuned on its own.
        let mut group = cfg.groups.iter().find(|g| g.id == parent).cloned().unwrap_or_else(|| Group::new(id, label.clone()));
        group.id = id;
        group.name = label.clone();
        group.routed_streams.clear();
        group.protected = false;
        group.cue = false;
        group.feeds_group = None;
        cfg.groups.push(group.clone());
        match cfg.profiles.iter_mut().find(|p| p.app_id == app_id) {
            Some(p) => p.preferred_group = Some(id),
            None => {
                let mut profile = AppProfile::new(app_id.clone(), label);
                profile.volume = volume;
                profile.preferred_group = Some(id);
                cfg.profiles.push(profile);
            }
        }
        let in_parent = cfg.groups.iter().find(|g| g.id == parent).map(|g| g.routed_streams.clone()).unwrap_or_default();
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::CreateGroup(group));
        let mut app_streams = crate::pw::streams_matching_app_id(&self.streams.lock().unwrap(), &app_id);
        app_streams.retain(|sid| *sid == stream_id || in_parent.contains(sid));
        for sid in app_streams {
            self.route_stream_to_group(sid, id);
        }
        self.emit("GroupChanged", &(id,));
        Ok(id)
    }

    fn route_stream_to_group(&mut self, stream_id: u32, group_id: u32) {
        // A manual move also becomes the app's preferred group: browsers open
        // a new stream (new id) on every pause/play, and without this each
        // one fell back into Default. Cue is a temporary solo, not a home.
        // ponytail: one group per app id, so the last move wins for every
        // browser tab without a site rule; per-site memory if that bites.
        let app = self.streams.lock().unwrap().get(&stream_id).map(|s| (s.app_id.clone(), s.name.clone()));
        let has_profile = app.as_ref().is_some_and(|(app_id, _)| self.state.lock().unwrap().profiles.iter().any(|p| &p.app_id == app_id));
        // wpctl, so before taking the config lock.
        let volume = if app.is_some() && !has_profile { get_stream_volume(stream_id) } else { 1.0 };
        let mut cfg = self.state.lock().unwrap();
        if let Some((app_id, label)) = app.filter(|_| cfg.groups.iter().any(|g| g.id == group_id && !g.cue)) {
            match cfg.profiles.iter_mut().find(|p| p.app_id == app_id) {
                Some(p) => p.preferred_group = Some(group_id),
                None => {
                    let mut profile = AppProfile::new(app_id, label);
                    profile.volume = volume;
                    profile.preferred_group = Some(group_id);
                    cfg.profiles.push(profile);
                }
            }
        }
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
        self.emit("GroupChanged", &(group_id,));
        self.emit("StreamListChanged", &());
    }

    /// Takes a stream back out of whatever *custom* group it's in and
    /// returns it to the protected "Default" group — never out of the
    /// mixer entirely (Section 7.2's "removed from the group" direction;
    /// `pw.rs::unroute_stream`'s own doc comment has the full story on why
    /// this doesn't bypass to raw hardware). The strip-from-every-group
    /// here is superseded by `Engine::route_and_persist`'s own (also
    /// correct) strip-then-add once the `Command` below is handled, but
    /// keeping it keeps this method's own persisted-state update accurate
    /// even before that round trip completes.
    fn unroute_stream(&mut self, stream_id: u32) {
        let mut cfg = self.state.lock().unwrap();
        for group in cfg.groups.iter_mut() {
            group.routed_streams.retain(|&id| id != stream_id);
        }
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::UnrouteStream { stream_id });
        self.emit("StreamListChanged", &());
    }

    fn set_gain(&mut self, group_id: u32, value: f32) {
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            group.gain = value;
        }
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::SetGain { group_id, value });
        self.emit("GroupChanged", &(group_id,));
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
        self.emit("GroupChanged", &(group_id,));
    }

    /// EQ level line (`Group::eq_level_db`): shifts the whole curve.
    fn set_eq_level(&mut self, group_id: u32, db: f32) {
        let db = db.clamp(-18.0, 18.0);
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            group.eq_level_db = db;
        }
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::SetEqLevel { group_id, db });
        self.emit("GroupChanged", &(group_id,));
    }

    fn get_eq_level(&self, group_id: u32) -> f32 {
        self.state.lock().unwrap().groups.iter().find(|g| g.id == group_id).map_or(0.0, |g| g.eq_level_db)
    }

    /// Pan (-1 left .. +1 right, 0 center) and mono/stereo for one group.
    fn set_pan(&mut self, group_id: u32, pan: f32, mono: bool) {
        let pan = if pan.is_finite() { pan.clamp(-1.0, 1.0) } else { 0.0 };
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            group.pan = pan;
            group.mono = mono;
        }
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::SetPan { group_id, pan, mono });
        self.emit("GroupChanged", &(group_id,));
    }

    /// (group_id, pan, mono) per group.
    fn list_group_pans(&self) -> Vec<(u32, f32, bool)> {
        self.state.lock().unwrap().groups.iter().map(|g| (g.id, g.pan, g.mono)).collect()
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
        self.emit("GroupChanged", &(group_id,));
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
        self.emit("GroupChanged", &(group_id,));
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
        self.emit("GroupChanged", &(group_id,));
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
        self.emit("GroupChanged", &(group_id,));
    }

    // --- Multi-output routing & delay compensation
    // (NewImprovenments/MTX_AUDIO_GROUPS_MULTI_OUTPUT.md,
    // MTX_MULTI_OUTPUT_DELAY_COMPENSATION.md) ---

    /// (output_id, name, description, kind, available, is_system_default,
    /// sample_rate, channels) per discovered PipeWire output
    /// (MTX_AUDIO_GROUPS_MULTI_OUTPUT.md §4's output model).
    #[allow(clippy::type_complexity)]
    fn list_outputs(&self) -> Vec<(String, String, String, String, bool, bool, u32, u32)> {
        let outputs = self.outputs.lock().unwrap();
        outputs
            .values()
            .map(|o| {
                (
                    o.id.clone(),
                    o.name.clone(),
                    o.description.clone(),
                    o.kind.as_str().to_owned(),
                    o.available(),
                    o.is_system_default,
                    o.sample_rate,
                    o.channels,
                )
            })
            .collect()
    }

    /// (volume, muted) for the real PipeWire sink behind this output —
    /// distinct from any group's own per-route gain onto it (a muted output
    /// stays silent no matter what a route's gain says, and nothing else in
    /// this app surfaces that). `None`-backed (disconnected) outputs report
    /// unity/unmuted since there's nothing live to query.
    fn get_output_volume(&self, output_id: String) -> (f32, bool) {
        // Lock released before shelling out: wpctl can wait on PipeWire,
        // which can wait on our engine thread, which can wait on this lock
        // (mark_output_unavailable) — holding it here deadlocked the daemon
        // and all of PipeWire on output solo.
        let node_id = { self.outputs.lock().unwrap().get(&output_id).and_then(|o| o.node_id) };
        match node_id {
            Some(node_id) => wpctl_get_volume(node_id),
            None => (1.0, false),
        }
    }

    /// Sets the real PipeWire sink's own volume via `wpctl` — same
    /// shell-out-to-the-already-trusted-tool choice `set_stream_muted`
    /// above and `pw.rs::set_stream_volume` already make for the per-stream
    /// equivalent.
    fn set_output_volume(&self, output_id: String, volume: f32) -> bool {
        let node_id = { self.outputs.lock().unwrap().get(&output_id).and_then(|o| o.node_id) };
        let Some(node_id) = node_id else { return false };
        wpctl()
            .args(["set-volume", &node_id.to_string(), &format!("{:.4}", volume.max(0.0))])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Mutes/unmutes the real PipeWire sink behind this output — the thing
    /// that actually silences playback regardless of any route's gain.
    fn set_output_muted(&self, output_id: String, muted: bool) -> bool {
        let node_id = { self.outputs.lock().unwrap().get(&output_id).and_then(|o| o.node_id) };
        let Some(node_id) = node_id else { return false };
        wpctl()
            .args(["set-mute", &node_id.to_string(), if muted { "1" } else { "0" }])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// (output_id, gain, enabled, mode, sync_mode, manual_delay_ms,
    /// sync_included) per configured route. Empty for a group with no
    /// explicit output routing — it still plays, just through the original
    /// default-sink behavior (MTX_AUDIO_GROUPS_MULTI_OUTPUT.md §7).
    #[allow(clippy::type_complexity)]
    fn list_group_output_routes(&self, group_id: u32) -> Vec<(String, f32, bool, String, String, f32, bool)> {
        let cfg = self.state.lock().unwrap();
        let Some(group) = cfg.groups.iter().find(|g| g.id == group_id) else { return Vec::new() };
        group
            .output_routes
            .iter()
            .map(|r| {
                (r.output_id.clone(), r.gain, r.enabled, r.mode.as_str().to_owned(), r.sync_mode.as_str().to_owned(), r.manual_delay_ms, r.sync_included)
            })
            .collect()
    }

    /// Adds or updates one explicit route (§7/§8) — always
    /// `RouteMode::Explicit` since this is a user-initiated call; the (not
    /// yet auto-wired) automatic default-output policy in `output.rs` is
    /// the only thing that would ever create an `Automatic`-mode route.
    fn set_group_output_route(&mut self, group_id: u32, output_id: String, gain: f32, enabled: bool) {
        if self.refuse_if_cue(group_id) {
            return;
        }
        let mut cfg = self.state.lock().unwrap();
        let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) else {
            tracing::warn!(group_id, "set_group_output_route for unknown group");
            return;
        };
        match group.output_routes.iter_mut().find(|r| r.output_id == output_id) {
            Some(existing) => {
                existing.gain = gain;
                existing.enabled = enabled;
            }
            None => {
                let mut route = GroupOutputRoute::new(output_id.clone(), RouteMode::Explicit);
                route.gain = gain;
                route.enabled = enabled;
                group.output_routes.push(route);
            }
        }
        let _ = cfg.save();
        drop(cfg);
        self.sync_group_routes(group_id);
        self.emit("GroupOutputRouteChanged", &(group_id, output_id));
    }

    // --- Master bus (Outputs tab "Master" routing target) ---
    // Same route shape/semantics as a group's own output_routes above —
    // gain/enabled/sync/manual-delay all mean exactly the same thing here,
    // just applied to the sum of every group instead of one. Mirrors those
    // four methods field-for-field rather than sharing code with them,
    // since "group_id" has no equivalent to thread through here.

    /// (output_id, gain, enabled, mode, sync_mode, manual_delay_ms,
    /// sync_included) per configured Master route. Empty means the Master
    /// bus plays nowhere yet — unlike a group, there's no implicit-default
    /// fallback for it (see `sync_master_routes`'s own doc comment).
    #[allow(clippy::type_complexity)]
    fn list_master_output_routes(&self) -> Vec<(String, f32, bool, String, String, f32, bool)> {
        let cfg = self.state.lock().unwrap();
        cfg.master_output_routes
            .iter()
            .map(|r| {
                (r.output_id.clone(), r.gain, r.enabled, r.mode.as_str().to_owned(), r.sync_mode.as_str().to_owned(), r.manual_delay_ms, r.sync_included)
            })
            .collect()
    }

    /// Adds or updates one explicit Master route — always
    /// `RouteMode::Explicit`, same reasoning as `set_group_output_route`.
    fn set_master_output_route(&mut self, output_id: String, gain: f32, enabled: bool) {
        let mut cfg = self.state.lock().unwrap();
        match cfg.master_output_routes.iter_mut().find(|r| r.output_id == output_id) {
            Some(existing) => {
                existing.gain = gain;
                existing.enabled = enabled;
            }
            None => {
                let mut route = GroupOutputRoute::new(output_id.clone(), RouteMode::Explicit);
                route.gain = gain;
                route.enabled = enabled;
                cfg.master_output_routes.push(route);
            }
        }
        let _ = cfg.save();
        drop(cfg);
        self.sync_master_routes();
        self.emit("MasterOutputRouteChanged", &(output_id,));
    }

    fn remove_master_output_route(&mut self, output_id: String) {
        let mut cfg = self.state.lock().unwrap();
        cfg.master_output_routes.retain(|r| r.output_id != output_id);
        let _ = cfg.save();
        drop(cfg);
        self.sync_master_routes();
        self.emit("MasterOutputRouteRemoved", &(output_id,));
    }

    /// Sets one Master route's synchronization mode and (for Manual) its
    /// delay — see `set_output_synchronization`'s own doc comment.
    fn set_master_output_synchronization(&mut self, output_id: String, sync_mode: String, manual_delay_ms: f32) {
        let Some(mode) = SyncMode::parse(&sync_mode) else {
            tracing::warn!(sync_mode, "unknown synchronization mode");
            return;
        };
        let mut cfg = self.state.lock().unwrap();
        if let Some(route) = cfg.master_output_routes.iter_mut().find(|r| r.output_id == output_id) {
            route.sync_mode = mode;
            route.manual_delay_ms = manual_delay_ms.max(0.0);
        }
        let _ = cfg.save();
        drop(cfg);
        self.sync_master_routes();
        self.emit("MasterOutputRouteChanged", &(output_id,));
    }

    fn remove_group_output_route(&mut self, group_id: u32, output_id: String) {
        if self.refuse_if_cue(group_id) {
            return;
        }
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            group.output_routes.retain(|r| r.output_id != output_id);
        }
        let _ = cfg.save();
        drop(cfg);
        self.sync_group_routes(group_id);
        self.emit("GroupOutputRouteRemoved", &(group_id, output_id));
    }

    /// Sets one route's synchronization mode and (for Manual) its delay
    /// (MTX_MULTI_OUTPUT_DELAY_COMPENSATION.md §4). `sync_mode` is one of
    /// "off"/"automatic"/"manual".
    fn set_output_synchronization(&mut self, group_id: u32, output_id: String, sync_mode: String, manual_delay_ms: f32) {
        let Some(mode) = SyncMode::parse(&sync_mode) else {
            tracing::warn!(sync_mode, "unknown synchronization mode");
            return;
        };
        if self.refuse_if_cue(group_id) {
            return;
        }
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            if let Some(route) = group.output_routes.iter_mut().find(|r| r.output_id == output_id) {
                route.sync_mode = mode;
                route.manual_delay_ms = manual_delay_ms.max(0.0);
            }
        }
        let _ = cfg.save();
        drop(cfg);
        self.sync_group_routes(group_id);
        self.emit("GroupOutputRouteChanged", &(group_id, output_id));
    }

    /// Clamped to `[0, ROUTE_DELAY_BUFFER_MS]` — a route's actual delay
    /// line is a fixed-size buffer sized to that constant at creation
    /// (`pw.rs`, MTX_MULTI_OUTPUT_DELAY_COMPENSATION.md §21's "avoid
    /// unnecessary allocations"), so a larger configured max would be
    /// silently truncated by the DSP layer while the persisted config
    /// still claimed the larger, unenforceable number — this keeps the two
    /// from disagreeing. Also sends the *clamped* value onward, not the
    /// raw input, so the persisted `Config` and the live engine always
    /// agree on the same number.
    fn set_max_compensation_ms(&mut self, value: f32) {
        let clamped = value.clamp(0.0, crate::pw::ROUTE_DELAY_BUFFER_MS);
        let mut cfg = self.state.lock().unwrap();
        cfg.max_compensation_ms = clamped;
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::SetMaxCompensationMs(clamped));
    }

    fn get_max_compensation_ms(&self) -> f32 {
        self.state.lock().unwrap().max_compensation_ms
    }

    // --- Cue (DJ-style solo bus) ---

    /// Marks/unmarks an output as a cue output. The permanent Cue group's
    /// routes follow this list exactly.
    fn set_cue_output(&mut self, output_id: String, enabled: bool) {
        let mut cfg = self.state.lock().unwrap();
        cfg.cue_outputs.retain(|o| o != &output_id);
        if enabled {
            cfg.cue_outputs.push(output_id.clone());
        }
        cfg.sync_cue_routes();
        let cue_id = cfg.cue_group_id();
        let _ = cfg.save();
        drop(cfg);
        if let Some(id) = cue_id {
            self.sync_group_routes(id);
        }
        self.emit("CueChanged", &(output_id,));
    }

    fn list_cue_outputs(&self) -> Vec<String> {
        self.state.lock().unwrap().cue_outputs.clone()
    }

    /// Id of the permanent Cue group (it always exists).
    fn get_cue_group_id(&self) -> u32 {
        self.state.lock().unwrap().cue_group_id().unwrap_or(u32::MAX)
    }

    /// Level (0..1) the rest of the mix drops to on cue outputs while the
    /// Cue group is playing.
    fn set_cue_duck_level(&mut self, value: f32) {
        let mut cfg = self.state.lock().unwrap();
        cfg.cue_duck_level = value.clamp(0.0, 1.0);
        let _ = cfg.save();
    }

    fn get_cue_duck_level(&self) -> f32 {
        self.state.lock().unwrap().cue_duck_level
    }

    // --- Settings tab (PROJECT_SPEC.md Section 7.5) ---

    /// Empty string means "no preference" (bare PipeWire autoconnect for
    /// every group's implicit default route, today's long-standing
    /// behavior) — same empty-string-means-none convention already used by
    /// `create_auto_route_rule`'s `app_id`/`service` filters.
    fn set_preferred_default_output(&mut self, output_id: String) {
        let mut cfg = self.state.lock().unwrap();
        cfg.preferred_default_output = if output_id.is_empty() { None } else { Some(output_id) };
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::SetPreferredDefaultOutput);
    }

    fn get_preferred_default_output(&self) -> String {
        self.state.lock().unwrap().preferred_default_output.clone().unwrap_or_default()
    }

    /// Clamped to the same `[10, 150]` range `sync_group_routes` itself
    /// enforces — kept here too so the *persisted* value and the one the
    /// engine will actually act on can't disagree (same reasoning as
    /// `set_max_compensation_ms`'s own clamp).
    fn set_buffer_ms(&mut self, value: f32) {
        let mut cfg = self.state.lock().unwrap();
        cfg.buffer_ms = value.clamp(10.0, 150.0);
        let _ = cfg.save();
    }

    fn get_buffer_ms(&self) -> f32 {
        self.state.lock().unwrap().buffer_ms
    }

    // --- Audio Scenes (NewImprovenments/01-audio-scenes.md) ---

    /// (id, name, description) per scene.
    fn list_scenes(&self) -> Vec<(u32, String, String)> {
        self.state.lock().unwrap().scenes.iter().map(|s| (s.id, s.name.clone(), s.description.clone())).collect()
    }

    /// Snapshots the current live config (every group's DSP/routing, plus
    /// which app is routed into which group by stable identity — §"Daemon
    /// Design") as a brand new scene.
    fn capture_current_as_scene(&mut self, name: String) -> u32 {
        let mut cfg = self.state.lock().unwrap();
        let id = cfg.scenes.iter().map(|s| s.id).max().map_or(0, |m| m + 1);
        let scene = capture_scene(&cfg, &self.streams, id, name, String::new());
        cfg.scenes.push(scene);
        let _ = cfg.save();
        drop(cfg);
        self.emit("SceneAdded", &(id,));
        id
    }

    /// Re-captures the current live config into an *existing* scene id,
    /// keeping its name/description — the "Update Scene" flow (tweak live
    /// settings, then save them back into the scene you started from,
    /// rather than editing a stored snapshot's fields in a separate form).
    fn update_scene(&mut self, scene_id: u32) -> bool {
        let mut cfg = self.state.lock().unwrap();
        let Some(existing) = cfg.scenes.iter().find(|s| s.id == scene_id) else { return false };
        let (name, description) = (existing.name.clone(), existing.description.clone());
        let scene = capture_scene(&cfg, &self.streams, scene_id, name, description);
        if let Some(slot) = cfg.scenes.iter_mut().find(|s| s.id == scene_id) {
            *slot = scene;
        }
        let _ = cfg.save();
        drop(cfg);
        self.emit("SceneUpdated", &(scene_id,));
        true
    }

    /// Returns `u32::MAX` (never a real assigned scene id, which always
    /// starts at 0) if `scene_id` doesn't exist — plain `0` would be
    /// ambiguous with a genuinely successful duplication landing on id 0.
    fn duplicate_scene(&mut self, scene_id: u32, new_name: String) -> u32 {
        let mut cfg = self.state.lock().unwrap();
        let id = cfg.scenes.iter().map(|s| s.id).max().map_or(0, |m| m + 1);
        let Some(source) = cfg.scenes.iter().find(|s| s.id == scene_id) else { return u32::MAX };
        let mut copy = source.clone();
        copy.id = id;
        copy.name = new_name;
        cfg.scenes.push(copy);
        let _ = cfg.save();
        drop(cfg);
        self.emit("SceneAdded", &(id,));
        id
    }

    fn delete_scene(&mut self, scene_id: u32) {
        let mut cfg = self.state.lock().unwrap();
        cfg.scenes.retain(|s| s.id != scene_id);
        let _ = cfg.save();
        drop(cfg);
        self.emit("SceneRemoved", &(scene_id,));
    }

    /// Applies the scene live and reports what did/didn't resolve —
    /// (applied_groups, missing_groups, applied_streams, pending_streams);
    /// `pending_streams` apps aren't running right now and are retried
    /// automatically once they start ("Recovery" section).
    /// Applies the scene live; apps not running right now are retried
    /// automatically once they start ("Recovery" section).
    fn activate_scene(&mut self, scene_id: u32) {
        self.send(Command::ActivateScene { scene_id });
        self.emit("SceneActivated", &(scene_id,));
    }

    // --- Per-App Audio Profiles (NewImprovenments/03-per-app-audio-profiles.md) ---

    /// Snapshots `stream_id`'s current group and PipeWire volume into a
    /// profile keyed by its stable app identity, so it comes back
    /// automatically next time that app starts (`pw.rs::on_global`).
    fn save_profile_for_stream(&mut self, stream_id: u32, label: String) -> bool {
        let Some(app_id) = self.streams.lock().unwrap().get(&stream_id).map(|s| s.app_id.clone()) else { return false };
        let volume = get_stream_volume(stream_id);
        let mut cfg = self.state.lock().unwrap();
        let preferred_group = cfg.groups.iter().find(|g| g.routed_streams.contains(&stream_id)).map(|g| g.id);
        let preferred_output = preferred_group
            .and_then(|gid| cfg.groups.iter().find(|g| g.id == gid))
            .and_then(|g| g.output_routes.first())
            .map(|r| r.output_id.clone());
        match cfg.profiles.iter_mut().find(|p| p.app_id == app_id) {
            Some(existing) => {
                existing.label = label;
                existing.volume = volume;
                existing.preferred_group = preferred_group;
                existing.preferred_output = preferred_output;
            }
            None => {
                let mut profile = AppProfile::new(app_id, label);
                profile.volume = volume;
                profile.preferred_group = preferred_group;
                profile.preferred_output = preferred_output;
                cfg.profiles.push(profile);
            }
        }
        let _ = cfg.save();
        true
    }

    // --- Rules engine (NewImprovenments/02-context-aware-auto-routing.md +
    // 07-audio-rules-engine.md, consolidated) ---

    /// (id, name, enabled, priority, cooldown_ms, trigger_summary,
    /// actions_summary) per rule — full structured trigger/condition/action
    /// editing isn't exposed over D-Bus (see `create_auto_route_rule`'s doc
    /// comment); this is enough for a list view plus enable/disable/delete.
    #[allow(clippy::type_complexity)]
    fn list_rules(&self) -> Vec<(u32, String, bool, i32, u64, String, String)> {
        let cfg = self.state.lock().unwrap();
        cfg.rules
            .iter()
            .map(|r| {
                (
                    r.id,
                    r.name.clone(),
                    r.enabled,
                    r.priority,
                    r.cooldown_ms,
                    crate::rules::describe_trigger(&r.trigger),
                    crate::rules::describe_actions(&r.actions, &cfg.groups),
                )
            })
            .collect()
    }

    /// One rule as JSON (`rules::Rule`'s serde shape), for the UI's rule
    /// builder to edit. Empty string for an unknown id.
    fn get_rule(&self, rule_id: u32) -> String {
        let cfg = self.state.lock().unwrap();
        cfg.rules.iter().find(|r| r.id == rule_id).and_then(|r| serde_json::to_string(r).ok()).unwrap_or_default()
    }

    /// Creates or replaces a rule from the rule builder's JSON. A `json`
    /// whose `id` matches an existing rule replaces it (keeping its place);
    /// anything else gets a fresh id. Returns the rule's id.
    fn save_rule(&mut self, json: String) -> zbus::fdo::Result<u32> {
        let mut rule: Rule = serde_json::from_str(&json).map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        crate::rules::validate(&rule).map_err(zbus::fdo::Error::InvalidArgs)?;
        let mut cfg = self.state.lock().unwrap();
        let missing_group = rule.actions.iter().find_map(|a| match a {
            Action::RouteTriggeringAppToGroup { group_id }
            | Action::RouteGroupToOutput { group_id, .. }
            | Action::SetGroupVolume { group_id, .. } => (!cfg.groups.iter().any(|g| g.id == *group_id)).then_some(*group_id),
            _ => None,
        });
        if let Some(id) = missing_group {
            return Err(zbus::fdo::Error::InvalidArgs(format!("no group {id}")));
        }
        if let Some(Action::ActivateScene { scene_id }) =
            rule.actions.iter().find(|a| matches!(a, Action::ActivateScene { scene_id } if !cfg.scenes.iter().any(|s| s.id == *scene_id)))
        {
            return Err(zbus::fdo::Error::InvalidArgs(format!("no scene {scene_id}")));
        }
        match cfg.rules.iter_mut().find(|r| r.id == rule.id) {
            Some(existing) => *existing = rule.clone(),
            None => {
                rule.id = cfg.rules.iter().map(|r| r.id).max().map_or(0, |m| m + 1);
                cfg.rules.push(rule.clone());
            }
        }
        let _ = cfg.save();
        drop(cfg);
        self.emit("RuleChanged", &(rule.id,));
        Ok(rule.id)
    }

    /// Recent rule firings, oldest first: (unix_secs, rule_name,
    /// trigger_summary, actions_summary, ok, message).
    #[allow(clippy::type_complexity)]
    fn rule_history(&self) -> Vec<(u64, String, String, String, bool, String)> {
        crate::rules::history()
            .into_iter()
            .map(|(at, e)| (at, e.rule_name, e.trigger_summary, e.actions_summary, e.ok, e.message))
            .collect()
    }

    /// Creates a rule from one of the fixed templates spec 02 itself names
    /// (`"route_app_to_group"`, `"route_app_to_group_and_apply_profile"`).
    /// Quickshell's quick-add uses this; the web UI uses `save_rule`.
    /// `app_id` must match `identity::app_identity`'s output exactly (the
    /// Inputs tab can supply this — it already knows each stream's app id).
    /// `service` instead matches the site/service name a browser tab's
    /// title exposes (e.g. "Udemy", "YouTube" — see
    /// `pw::extract_service_name`), case-insensitively — the way to target
    /// one tab/site without knowing PipeWire's raw identity string, since
    /// every tab of the same browser otherwise shares one `app_id`.
    /// Returns `u32::MAX` (never a real assigned rule id, which always
    /// starts at 0) for an unknown `template` — plain `0` would be
    /// ambiguous with a genuinely successful call landing on id 0.
    fn create_auto_route_rule(
        &mut self,
        name: String,
        template: String,
        app_id: String,
        service: String,
        group_id: u32,
    ) -> u32 {
        let actions = match template.as_str() {
            "route_app_to_group" => vec![Action::RouteTriggeringAppToGroup { group_id }],
            "route_app_to_group_and_apply_profile" => {
                vec![Action::RouteTriggeringAppToGroup { group_id }, Action::ApplyProfileForTriggeringApp]
            }
            _ => {
                tracing::warn!(template, "unknown rule template");
                return u32::MAX;
            }
        };
        let mut cfg = self.state.lock().unwrap();
        let id = cfg.rules.iter().map(|r| r.id).max().map_or(0, |m| m + 1);
        let app_id_filter: Option<AppId> = if app_id.is_empty() { None } else { Some(app_id) };
        let service_filter = if service.is_empty() { None } else { Some(service) };
        let rule = Rule {
            id,
            name,
            enabled: true,
            priority: 0,
            cooldown_ms: 2_000,
            trigger: Trigger::AppStarted { app_id: app_id_filter, service: service_filter },
            conditions: Vec::new(),
            actions,
        };
        cfg.rules.push(rule);
        let _ = cfg.save();
        drop(cfg);
        self.emit("RuleChanged", &(id,));
        id
    }

    /// Creates a device-availability-gated route rule — spec 02's other
    /// named example ("USB headset connected → move communication apps to
    /// it"): when `output_id` connects, route `group_id` onward to that
    /// output at `route_gain`. Operates on the whole group rather than a
    /// single app (an `AppStarted`-shaped trigger would be the wrong
    /// primitive for "device connected" — this uses `DeviceConnected`
    /// directly); route every relevant app into `group_id` first (manually,
    /// via a profile, or via `create_auto_route_rule`) for the "move
    /// communication apps to it" framing the spec describes.
    fn create_device_connected_rule(&mut self, name: String, output_id: String, group_id: u32, route_gain: f32) -> u32 {
        let mut cfg = self.state.lock().unwrap();
        let id = cfg.rules.iter().map(|r| r.id).max().map_or(0, |m| m + 1);
        let rule = Rule {
            id,
            name,
            enabled: true,
            priority: 0,
            cooldown_ms: 5_000,
            trigger: Trigger::DeviceConnected { output_id: Some(output_id.clone()) },
            conditions: vec![Condition::OutputAvailable(output_id.clone())],
            actions: vec![Action::RouteGroupToOutput { group_id, output_id, gain: route_gain }],
        };
        cfg.rules.push(rule);
        let _ = cfg.save();
        drop(cfg);
        self.emit("RuleChanged", &(id,));
        id
    }

    fn set_rule_enabled(&mut self, rule_id: u32, enabled: bool) {
        let mut cfg = self.state.lock().unwrap();
        if let Some(r) = cfg.rules.iter_mut().find(|r| r.id == rule_id) {
            r.enabled = enabled;
        }
        let _ = cfg.save();
        drop(cfg);
        self.emit("RuleChanged", &(rule_id,));
    }

    fn delete_rule(&mut self, rule_id: u32) {
        let mut cfg = self.state.lock().unwrap();
        cfg.rules.retain(|r| r.id != rule_id);
        let _ = cfg.save();
        drop(cfg);
        self.emit("RuleChanged", &(rule_id,));
    }

    // --- Intelligent Audio Ducking (NewImprovenments/04-intelligent-audio-ducking.md) ---

    #[allow(clippy::type_complexity)]
    fn list_ducking_rules(&self) -> Vec<(u32, bool, u32, Vec<u32>, f32, f32, f32, f32, f32)> {
        self.state
            .lock()
            .unwrap()
            .ducking_rules
            .iter()
            .map(|r| (r.id, r.enabled, r.trigger_group, r.target_groups.clone(), r.reduction_db, r.attack_ms, r.hold_ms, r.release_ms, r.threshold_db))
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn create_ducking_rule(
        &mut self,
        trigger_group: u32,
        target_groups: Vec<u32>,
        reduction_db: f32,
        attack_ms: f32,
        hold_ms: f32,
        release_ms: f32,
        threshold_db: f32,
    ) -> u32 {
        let mut cfg = self.state.lock().unwrap();
        let id = cfg.ducking_rules.iter().map(|r| r.id).max().map_or(0, |m| m + 1);
        cfg.ducking_rules.push(DuckingRule { id, enabled: true, trigger_group, target_groups, reduction_db, attack_ms, hold_ms, release_ms, threshold_db });
        let _ = cfg.save();
        drop(cfg);
        // No `Command` to forward here (unlike everything else in this
        // file): `Engine::evaluate_ducking` reads `cfg.ducking_rules` fresh
        // from this same shared `Config` on every ~20ms tick, so a change
        // takes effect within one tick with no explicit sync message —
        // the signal below is purely for other UI clients' benefit.
        self.emit("DuckingRuleChanged", &(id,));
        id
    }

    fn set_ducking_rule_enabled(&mut self, rule_id: u32, enabled: bool) {
        let mut cfg = self.state.lock().unwrap();
        if let Some(r) = cfg.ducking_rules.iter_mut().find(|r| r.id == rule_id) {
            r.enabled = enabled;
        }
        let _ = cfg.save();
        drop(cfg);
        self.emit("DuckingRuleChanged", &(rule_id,));
    }

    fn delete_ducking_rule(&mut self, rule_id: u32) {
        let mut cfg = self.state.lock().unwrap();
        cfg.ducking_rules.retain(|r| r.id != rule_id);
        let _ = cfg.save();
        drop(cfg);
        self.emit("DuckingRuleChanged", &(rule_id,));
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

    /// (id, name, service, is_actively_playing, app_id) per detected app
    /// stream. `service` is the extracted site/service name (e.g. "YouTube"
    /// out of a Firefox tab's full title) — empty when the stream doesn't
    /// expose one, in which case the UI just shows `name`. `app_id` is the
    /// raw identity string a rule's "App id" filter must match exactly
    /// (`identity::app_identity`'s output) — exposed so the UI can offer it
    /// as a pick rather than making someone guess/read daemon logs for it.
    /// (stream_id, peak 0..1) per app stream — the Inputs tab's per-app
    /// level meters. Cheap: reads atomics the meter taps keep current.
    fn get_stream_levels(&self) -> Vec<(u32, f32)> {
        let streams = self.streams.lock().unwrap();
        streams.values().map(|s| (s.id, f32::from_bits(s.peak.load(std::sync::atomic::Ordering::Relaxed)))).collect()
    }

    /// L/R isolate for one app stream: "L" or "R" sends all of it to that
    /// channel of its group, anything else puts it back to stereo. Not
    /// persisted: it lasts as long as the stream.
    fn set_stream_side(&mut self, stream_id: u32, side: String) {
        let side = match side.as_str() {
            "L" => Some("FL"),
            "R" => Some("FR"),
            _ => None,
        };
        if let Some(s) = self.streams.lock().unwrap().get_mut(&stream_id) {
            s.side = side;
        }
        self.send(Command::RelinkStream { stream_id });
    }

    /// (stream_id, "L" | "R") for every stream that isn't plain stereo.
    fn list_stream_sides(&self) -> Vec<(u32, String)> {
        let streams = self.streams.lock().unwrap();
        streams.values().filter_map(|s| s.side.map(|c| (s.id, if c == "FL" { "L" } else { "R" }.to_owned()))).collect()
    }

    fn list_streams(&self) -> Vec<(u32, String, String, bool, String)> {
        let streams = self.streams.lock().unwrap();
        streams.values().map(|s| (s.id, s.name.clone(), s.service.clone(), s.running, s.app_id.clone())).collect()
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
        wpctl()
            .args(["set-mute", &stream_id.to_string(), if muted { "1" } else { "0" }])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    // --- MTX Doctor (NewImprovenments/08-pipewire-diagnostics-doctor.md) ---

    /// Overall score ("ok"/"warning"/"error") plus (check_name, severity,
    /// message) per check — everything the Doctor page needs in one call.
    fn run_health_checks(&self) -> (String, Vec<(String, String, String)>) {
        let cfg = self.state.lock().unwrap();
        let live = self.live.lock().unwrap();
        let outputs = self.outputs.lock().unwrap();
        let configured_group_ids: Vec<u32> = cfg.groups.iter().map(|g| g.id).collect();
        let live_group_ids: Vec<u32> = live.keys().copied().collect();
        let routes_pointing_at_unavailable_outputs = cfg
            .groups
            .iter()
            .flat_map(|g| &g.output_routes)
            .filter(|r| outputs.get(&r.output_id).is_none_or(|o| !o.available()))
            .count();
        let underrun_count = self.metrics.underruns.load(std::sync::atomic::Ordering::Relaxed);
        let underrun_rate_per_sec = {
            let mut last = self.last_underrun_check.lock().unwrap();
            let elapsed = last.1.elapsed().as_secs_f32().max(0.001);
            let rate = underrun_count.saturating_sub(last.0) as f32 / elapsed;
            *last = (underrun_count, std::time::Instant::now());
            rate
        };
        let inputs = crate::doctor::HealthInputs {
            configured_group_ids,
            live_group_ids,
            buffer_frames: self.metrics.buffer_frames.load(std::sync::atomic::Ordering::Relaxed),
            underrun_count,
            underrun_rate_per_sec,
            total_outputs: outputs.len(),
            available_outputs: outputs.values().filter(|o| o.available()).count(),
            routes_pointing_at_unavailable_outputs,
        };
        let checks = crate::doctor::run_checks(&inputs);
        let score = crate::doctor::overall_score(&checks).as_str().to_owned();
        let rows = checks.into_iter().map(|c| (c.name, c.severity.as_str().to_owned(), c.message)).collect();
        (score, rows)
    }

    // --- Visual Signal-Flow Graph (NewImprovenments/06-visual-signal-flow-graph.md) ---

    /// (node_id, label, kind, state) nodes and (from, to) edges — a fresh
    /// snapshot computed from the same state every other tab reads, not a
    /// separately-tracked graph model (`graph.rs`'s module doc).
    #[allow(clippy::type_complexity)]
    fn get_graph(&self) -> (Vec<(String, String, String, String)>, Vec<(String, String)>) {
        let cfg = self.state.lock().unwrap();
        let streams: Vec<AppStream> = self.streams.lock().unwrap().values().cloned().collect();
        let outputs: Vec<OutputDevice> = self.outputs.lock().unwrap().values().cloned().collect();
        let graph = crate::graph::build_graph(&cfg.groups, &streams, &outputs);
        let nodes = graph.nodes.into_iter().map(|n| (n.id, n.label, n.kind, n.state)).collect();
        let edges = graph.edges.into_iter().map(|e| (e.from, e.to)).collect();
        (nodes, edges)
    }

    // --- Per-App/Per-Group Recording (NewImprovenments/09-per-app-recording.md) ---

    /// `format` is `"wav16"`, `"wav32"` or `"flac"` (24-bit). Returns the new recording's
    /// id, or `u32::MAX` if the format/group was invalid.
    fn start_recording(&mut self, group_id: u32, path: String, format: String) -> u32 {
        let format = match format.as_str() {
            "wav16" => crate::recording::RecordingFormat::WavPcm16,
            "wav32" => crate::recording::RecordingFormat::WavFloat32,
            "flac" => crate::recording::RecordingFormat::Flac24,
            other => {
                tracing::warn!(format = other, "unknown recording format");
                return u32::MAX;
            }
        };
        if !self.state.lock().unwrap().groups.iter().any(|g| g.id == group_id) {
            tracing::warn!(group_id, "start_recording for unknown group");
            return u32::MAX;
        }
        let id = self.next_recording_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.send(Command::StartRecording { id, group_id, path: std::path::PathBuf::from(path), format });
        self.emit("RecordingChanged", &(id,));
        id
    }

    fn stop_recording(&mut self, id: u32) {
        self.send(Command::StopRecording { id });
        self.emit("RecordingChanged", &(id,));
    }

    /// (id, group_id, path, elapsed_secs, frames_written) per active
    /// recording — elapsed/frames are read lock-free off the writer
    /// thread's own live counters, not polled from it.
    #[allow(clippy::type_complexity)]
    fn list_recordings(&self) -> Vec<(u32, u32, String, f32, u64)> {
        self.recording_status
            .lock()
            .unwrap()
            .iter()
            .map(|(&id, s)| (id, s.group_id, s.path.clone(), s.elapsed_secs(), s.frames_written()))
            .collect()
    }

    // --- Plugin / DSP Architecture (NewImprovenments/10-plugin-dsp-architecture.md) ---

    /// (type_id, display_name, param_names) per available built-in plugin
    /// type. Parameter min/max/default aren't included in this tuple shape
    /// to keep it flat for D-Bus — a UI wanting those can hardcode sane
    /// slider ranges per known `type_id`, same pragmatic tradeoff already
    /// made elsewhere in this API (e.g. compressor/gate params are typed
    /// positionally, not self-describing either).
    /// (type_id, display_name, [(param_name, min, max, default)]) — lets
    /// the UI draw a knob per param and reset it without hardcoding them.
    fn list_available_plugins(&self) -> Vec<(String, String, Vec<(String, f32, f32, f32)>)> {
        crate::plugins::builtin_registry()
            .into_iter()
            .map(|m| (m.type_id, m.display_name, m.params.into_iter().map(|p| (p.name, p.min, p.max, p.default)).collect()))
            .collect()
    }

    /// (index, plugin_type, enabled, param_names, param_values) per slot in
    /// `group_id`'s chain, in processing order.
    #[allow(clippy::type_complexity)]
    fn list_group_plugins(&self, group_id: u32) -> Vec<(u32, String, bool, Vec<String>, Vec<f32>)> {
        let cfg = self.state.lock().unwrap();
        let Some(group) = cfg.groups.iter().find(|g| g.id == group_id) else { return Vec::new() };
        group
            .plugins
            .iter()
            .enumerate()
            .map(|(i, inst)| {
                let names = inst.params.iter().map(|(n, _)| n.clone()).collect();
                let values = inst.params.iter().map(|(_, v)| *v).collect();
                (i as u32, inst.plugin_type.clone(), inst.enabled, names, values)
            })
            .collect()
    }

    fn add_group_plugin(&mut self, group_id: u32, plugin_type: String) {
        let mut cfg = self.state.lock().unwrap();
        let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) else { return };
        let defaults = crate::plugins::builtin_registry()
            .into_iter()
            .find(|m| m.type_id == plugin_type)
            .map(|m| m.params.into_iter().map(|p| (p.name, p.default)).collect())
            .unwrap_or_default();
        group.plugins.push(crate::plugins::PluginInstance { plugin_type, enabled: true, params: defaults });
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::SyncGroupPlugins { group_id });
        self.emit("GroupPluginsChanged", &(group_id,));
    }

    fn remove_group_plugin(&mut self, group_id: u32, index: u32) {
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            if (index as usize) < group.plugins.len() {
                group.plugins.remove(index as usize);
            }
        }
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::SyncGroupPlugins { group_id });
        self.emit("GroupPluginsChanged", &(group_id,));
    }

    fn set_group_plugin_bypass(&mut self, group_id: u32, index: u32, bypassed: bool) {
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            if let Some(slot) = group.plugins.get_mut(index as usize) {
                slot.enabled = !bypassed;
            }
        }
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::SyncGroupPlugins { group_id });
        self.emit("GroupPluginsChanged", &(group_id,));
    }

    fn set_group_plugin_param(&mut self, group_id: u32, index: u32, param_name: String, value: f32) {
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            if let Some(slot) = group.plugins.get_mut(index as usize) {
                match slot.params.iter_mut().find(|(n, _)| n == &param_name) {
                    Some((_, v)) => *v = value,
                    None => slot.params.push((param_name, value)),
                }
            }
        }
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::SyncGroupPlugins { group_id });
        self.emit("GroupPluginsChanged", &(group_id,));
    }

    /// Moves the plugin at `from_index` to `to_index`, shifting the rest —
    /// spec's "Allow reordering processors".
    fn reorder_group_plugin(&mut self, group_id: u32, from_index: u32, to_index: u32) {
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            let (from, to) = (from_index as usize, to_index as usize);
            if from < group.plugins.len() && to < group.plugins.len() {
                let item = group.plugins.remove(from);
                group.plugins.insert(to, item);
            }
        }
        let _ = cfg.save();
        drop(cfg);
        self.send(Command::SyncGroupPlugins { group_id });
        self.emit("GroupPluginsChanged", &(group_id,));
    }
}

/// Builds a `Scene` snapshot of the current live config — shared by
/// `capture_current_as_scene` and `update_scene` so "capture" and
/// "re-capture into an existing id" can't drift apart.
fn capture_scene(cfg: &Config, streams: &Arc<Mutex<HashMap<u32, AppStream>>>, id: u32, name: String, description: String) -> Scene {
    let streams = streams.lock().unwrap();
    // Cue holds only temporarily soloed streams — not a placement to save.
    let routed: Vec<(u32, crate::identity::AppId, u32)> = cfg
        .groups
        .iter()
        .filter(|g| !g.cue)
        .flat_map(|g| g.routed_streams.iter().map(move |&sid| (sid, g.id)))
        .filter_map(|(sid, gid)| streams.get(&sid).map(|s| (sid, s.app_id.clone(), gid)))
        .collect();
    let (stream_placements, stream_assignments) = crate::scenes::build_assignments(&routed);
    Scene { id, name, description, master_gain: cfg.master_gain, groups: cfg.groups.clone(), stream_assignments, stream_placements }
}

/// Reads a raw PipeWire stream's current volume via `wpctl` — the read
/// counterpart to `set_stream_volume` (`pw.rs`), used when saving a profile
/// so it captures whatever the user actually has it set to right now.
fn get_stream_volume(stream_id: u32) -> f32 {
    wpctl()
        .args(["get-volume", &stream_id.to_string()])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|s| s.split_whitespace().nth(1).and_then(|v| v.parse::<f32>().ok()))
        .unwrap_or(1.0)
}

/// Reads a PipeWire object's volume *and* mute state via `wpctl get-volume`,
/// whose output looks like "Volume: 0.65" or "Volume: 0.65 [MUTED]" — the
/// read counterpart to `set_output_volume`/`set_output_muted` above.
fn wpctl_get_volume(node_id: u32) -> (f32, bool) {
    wpctl()
        .args(["get-volume", &node_id.to_string()])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| (s.split_whitespace().nth(1).and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0), s.contains("[MUTED]")))
        .unwrap_or((1.0, false))
}

/// Blocks the calling thread forever serving the D-Bus interface. Meant to
/// be run on its own `std::thread`.
#[allow(clippy::too_many_arguments)]
pub fn serve(
    state: Arc<Mutex<Config>>,
    streams: Arc<Mutex<HashMap<u32, AppStream>>>,
    cmd_tx: pipewire::channel::Sender<Command>,
    live: Arc<Mutex<HashMap<u32, Arc<GroupLive>>>>,
    metrics: Arc<Metrics>,
    outputs: Arc<Mutex<HashMap<OutputId, OutputDevice>>>,
    recording_status: Arc<Mutex<HashMap<u32, crate::pw::RecordingStatus>>>,
) {
    // Connect, register the interface, *then* claim the name, so no call
    // can reach the name before `Daemon` is there to answer it. `Daemon`
    // needs a connection clone to emit signals, which is why this isn't the
    // single fluent `Builder::serve_at(...).name(...)` chain.
    let conn = match zbus::blocking::connection::Builder::session().and_then(|b| b.build()) {
        Ok(conn) => conn,
        Err(err) => {
            tracing::error!(%err, "failed to start D-Bus service");
            return;
        }
    };
    let daemon = Daemon::new(state, streams, cmd_tx, live, metrics, outputs, recording_status, conn.clone());
    if let Err(err) = conn.object_server().at("/org/sonicforge/Daemon", daemon) {
        tracing::error!(%err, "failed to register D-Bus interface");
        return;
    }
    if let Err(err) = conn.request_name("org.sonicforge.Daemon") {
        tracing::error!(%err, "failed to claim org.sonicforge.Daemon");
        return;
    }
    tracing::info!("D-Bus service org.sonicforge.Daemon registered");
    // The connection's own executor thread(s) keep serving requests as long
    // as `conn` stays alive. This thread waits for a newer daemon to take
    // the name (zbus claims it with replace-existing): the old one then shuts
    // down the same way as on SIGTERM, instead of fighting over the routing.
    // A clean exit, so the unit's Restart=on-failure leaves it down.
    let name_lost = zbus::blocking::fdo::DBusProxy::new(&conn).and_then(|p| p.receive_name_lost());
    match name_lost {
        Ok(signals) => {
            for signal in signals {
                if signal.args().is_ok_and(|a| a.name() == "org.sonicforge.Daemon") {
                    tracing::info!("another sonicforge-daemon took over org.sonicforge.Daemon, shutting down");
                    unsafe { libc::kill(libc::getpid(), libc::SIGTERM) };
                    break;
                }
            }
        }
        Err(err) => tracing::warn!(%err, "can't watch for D-Bus name loss"),
    }
    loop {
        std::thread::park();
    }
}

/// `wpctl` capped at 2s: every D-Bus method shares one executor thread, so
/// a wpctl stuck behind a busy PipeWire must not stall the whole interface.
fn wpctl() -> std::process::Command {
    let mut cmd = std::process::Command::new("timeout");
    cmd.args(["2", "wpctl"]);
    cmd
}
