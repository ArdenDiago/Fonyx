//! PipeWire engine: connects to PipeWire, tracks per-app audio streams, and
//! owns one capture+playback stream pair per group (PROJECT_SPEC.md Section
//! 4/6, build-order stages 1-3/5/6).
//!
//! Everything in this file — registry events, `Command` handling, and every
//! stream's `process()` callback — runs on the single thread that calls
//! `run()` (pipewire-rs's `pw_main_loop` wrapper dispatches all of it there;
//! there's no separate OS thread per stream the way `pw_thread_loop` would
//! give you). That keeps the control/audio split simple, but it does mean a
//! `CreateGroup`/`RouteStreamToGroup` command shares a thread with already-
//! running groups' audio callbacks.
//!
//! ponytail: single mainloop thread means an infrequent control op (create/
//! route) could in principle delay another group's audio callback by a
//! sliver — acceptable for "a handful of apps" desktop use (Section 10).
//! Upgrade path if xruns show up in practice: move to `pw::thread_loop`
//! so control and audio callbacks run on genuinely separate threads.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use pipewire as pw;
use pw::keys;
use pw::properties::properties;
use pw::registry::GlobalObject;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Object, Pod, Value};
use pw::spa::utils::dict::DictRef;
use pw::spa::utils::{Direction, SpaTypes};

use crate::config::Config;
use crate::dsp::biquad::{Biquad, EqBand, FilterSlope};
use crate::dsp::dynamics::{CompressorParams, DynamicsProcessor, GateParams};
use crate::dsp::smoothed::Smoothed;
use crate::dsp::spectrum::SpectrumAnalyzer;
use crate::group::Group;

const SAMPLE_RATE: f32 = 48_000.0;
const CHANNELS: usize = 2;
const PARAM_RAMP_MS: f32 = 15.0;
/// ~170ms of stereo headroom at 48kHz between the capture and playback
/// callbacks, which may run with different buffer sizes/timing.
const RING_CAPACITY: usize = 1 << 14;

/// A single detected application audio output stream.
#[derive(Debug, Clone)]
pub struct AppStream {
    pub id: u32,
    pub name: String,
    /// Extracted from the stream's `media.name` (e.g. a browser tab's
    /// title) via `extract_service_name` — "YouTube" out of a full video
    /// title, "WhatsApp" as-is, etc. Empty when the stream doesn't set
    /// `media.name` to anything beyond its own app name.
    pub service: String,
    /// Whether PipeWire currently has this node in `RUNNING` state, i.e. it
    /// is actively producing audio right now — not just open/idle. Backs
    /// the Inputs tab's "which tab is actually playing" indicator (Section
    /// 7.2).
    pub running: bool,
}

/// Pulls a short "service/site name" out of a stream's `media.name` (a
/// browser tab title, most often) so the Inputs tab can show "YouTube"
/// instead of a full, ever-changing video title. Two patterns cover what's
/// actually observed live: a leading unread-count badge ("(1) ...", "(57)
/// WhatsApp") gets stripped, then the trailing " - Site" segment (browser
/// tabs conventionally suffix the site name) is taken as the service name;
/// with no " - " separator at all, the whole (count-stripped) title already
/// *is* the service name (single-page apps like WhatsApp Web/Spotify Web
/// just set their own name as the title).
fn extract_service_name(media_name: &str) -> Option<String> {
    let stripped = media_name
        .strip_prefix('(')
        .and_then(|rest| rest.split_once(')'))
        .map(|(_, rest)| rest.trim_start())
        .unwrap_or(media_name);
    if stripped.is_empty() {
        return None;
    }
    let service = stripped.rsplit(" - ").next().unwrap_or(stripped).trim();
    if service.is_empty() { None } else { Some(service.to_owned()) }
}

/// Engine-wide health counters for the Home tab (PROJECT_SPEC.md Section
/// 7.1/8's `MetricsUpdated`). Lock-free, same `AtomicU32`-bit-cast-f32 idiom
/// as `dsp::smoothed::Smoothed` — written from the audio/sysinfo threads,
/// read from the D-Bus thread.
///
/// ponytail: `underruns` counts our own capture->playback ring buffer
/// starving (a real, locally-measured signal), not the PipeWire graph's own
/// driver xrun count (that lives on the driver node's profiler data, which
/// needs a profiler subscription this engine doesn't have). Good enough to
/// tell "audio is glitching" from "it isn't"; upgrade path is a `pw::Context`
/// profiler listener if the distinction ever matters.
#[derive(Default)]
pub struct Metrics {
    pub buffer_frames: AtomicU32,
    pub underruns: AtomicU64,
    cpu_pct_bits: AtomicU32,
    ram_pct_bits: AtomicU32,
}

impl Metrics {
    pub fn set_cpu_pct(&self, v: f32) {
        self.cpu_pct_bits.store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn cpu_pct(&self) -> f32 {
        f32::from_bits(self.cpu_pct_bits.load(Ordering::Relaxed))
    }
    pub fn set_ram_pct(&self, v: f32) {
        self.ram_pct_bits.store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn ram_pct(&self) -> f32 {
        f32::from_bits(self.ram_pct_bits.load(Ordering::Relaxed))
    }
}

/// Per-group live values for the Home/MTX Panel meters: post-chain peak
/// level (L/R) and current compressor gain reduction, all f32-bits-in-
/// `AtomicU32` so the capture callback can publish them without a lock.
///
/// Also the underrun gate: a group's *playback* stream is autoconnected to
/// real hardware, so PipeWire's graph driver calls its `process()` every
/// quantum regardless of whether anything is routed into the group yet —
/// but the *capture* stream (an unlinked `Audio/Sink`) only gets buffers to
/// dequeue once some app stream is actually linked in. Without this gate,
/// an idle/freshly-created group's ring buffer starves from the moment it
/// exists, and `underruns` (Metrics) counts millions of "misses" that are
/// really just silence — not the audible glitches the metric is meant to
/// flag. `has_input` only ever flips true once, on the first real capture
/// buffer; it doesn't flip back off if the route is later removed, which
/// is an acceptable ponytail: upgrade to a decaying/timestamped flag if
/// "route removed, then underrun count keeps climbing" ever matters.
pub struct GroupLive {
    level_l_bits: AtomicU32,
    level_r_bits: AtomicU32,
    gr_db_bits: AtomicU32,
    gate_gr_db_bits: AtomicU32,
    has_input: AtomicBool,
    /// One `AtomicU32` (f32 bits) per spectrum bin — same lock-free idiom
    /// as everything else here, just an array of it instead of one value.
    spectrum: [AtomicU32; crate::dsp::spectrum::N_BINS],
}

impl Default for GroupLive {
    fn default() -> Self {
        Self {
            level_l_bits: AtomicU32::new(0),
            level_r_bits: AtomicU32::new(0),
            gr_db_bits: AtomicU32::new(0),
            gate_gr_db_bits: AtomicU32::new(0),
            has_input: AtomicBool::new(false),
            // Floor, not 0.0 — a bin that's never had a real FFT computed
            // (idle group, nothing routed in yet) must read as silence, not
            // as a misleadingly "loud" 0dB flat line on any UI spectrum
            // graph.
            spectrum: std::array::from_fn(|_| AtomicU32::new(crate::dsp::spectrum::FLOOR_DB.to_bits())),
        }
    }
}

impl GroupLive {
    fn publish(&self, level_l: f32, level_r: f32, gr_db: f32, gate_gr_db: f32) {
        self.level_l_bits.store(level_l.to_bits(), Ordering::Relaxed);
        self.level_r_bits.store(level_r.to_bits(), Ordering::Relaxed);
        self.gr_db_bits.store(gr_db.to_bits(), Ordering::Relaxed);
        self.gate_gr_db_bits.store(gate_gr_db.to_bits(), Ordering::Relaxed);
    }

    fn publish_spectrum(&self, bins: &[f32; crate::dsp::spectrum::N_BINS]) {
        for (slot, &db) in self.spectrum.iter().zip(bins.iter()) {
            slot.store(db.to_bits(), Ordering::Relaxed);
        }
    }

    pub fn read(&self) -> (f32, f32, f32, f32) {
        (
            f32::from_bits(self.level_l_bits.load(Ordering::Relaxed)),
            f32::from_bits(self.level_r_bits.load(Ordering::Relaxed)),
            f32::from_bits(self.gr_db_bits.load(Ordering::Relaxed)),
            f32::from_bits(self.gate_gr_db_bits.load(Ordering::Relaxed)),
        )
    }

    pub fn read_spectrum(&self) -> [f32; crate::dsp::spectrum::N_BINS] {
        std::array::from_fn(|i| f32::from_bits(self.spectrum[i].load(Ordering::Relaxed)))
    }

    fn mark_active(&self) {
        self.has_input.store(true, Ordering::Relaxed);
    }

    fn has_input(&self) -> bool {
        self.has_input.load(Ordering::Relaxed)
    }
}

/// Control-thread -> pw-thread messages (PROJECT_SPEC.md Section 8's D-Bus
/// methods, minus the read-only ones which just read shared state directly).
pub enum Command {
    CreateGroup(Group),
    DeleteGroup(u32),
    RouteStream { stream_id: u32, group_id: u32 },
    UnrouteStream { stream_id: u32 },
    SetGain { group_id: u32, value: f32 },
    SetMasterGain(f32),
    SetEqBand { group_id: u32, index: usize, band: EqBand },
    SetHighpass { group_id: u32, filter: FilterSlope },
    SetLowpass { group_id: u32, filter: FilterSlope },
    SetCompressor { group_id: u32, params: CompressorParams },
    SetGate { group_id: u32, params: GateParams },
}

/// Live parameters for one EQ band, shared (via `Arc`) between the control
/// thread (writes targets) and that group's capture-stream process closure
/// (reads/ramps them). Trig-heavy `Biquad` coefficient rebuilds happen once
/// per buffer in the process closure, not per sample — see `dsp::biquad`.
struct EqBandRt {
    freq: Smoothed,
    gain_db: Smoothed,
    q: Smoothed,
    enabled: AtomicBool,
}

impl EqBandRt {
    fn new(band: &EqBand) -> Self {
        Self {
            freq: Smoothed::new(band.freq_hz, SAMPLE_RATE, PARAM_RAMP_MS),
            gain_db: Smoothed::new(band.gain_db, SAMPLE_RATE, PARAM_RAMP_MS),
            q: Smoothed::new(band.q, SAMPLE_RATE, PARAM_RAMP_MS),
            enabled: AtomicBool::new(band.enabled),
        }
    }

    fn set(&self, band: &EqBand) {
        self.freq.set_target(band.freq_hz);
        self.gain_db.set_target(band.gain_db);
        self.q.set_target(band.q);
        self.enabled.store(band.enabled, Ordering::Relaxed);
    }

    fn snapshot_current(&self) -> EqBand {
        EqBand {
            freq_hz: self.freq.current(),
            gain_db: self.gain_db.current(),
            q: self.q.current(),
            enabled: self.enabled.load(Ordering::Relaxed),
        }
    }
}

struct FilterRt {
    freq: Smoothed,
    q: Smoothed,
    enabled: AtomicBool,
}

impl FilterRt {
    fn new(f: &FilterSlope) -> Self {
        Self {
            freq: Smoothed::new(f.freq_hz, SAMPLE_RATE, PARAM_RAMP_MS),
            q: Smoothed::new(f.q, SAMPLE_RATE, PARAM_RAMP_MS),
            enabled: AtomicBool::new(f.enabled),
        }
    }

    fn set(&self, f: &FilterSlope) {
        self.freq.set_target(f.freq_hz);
        self.q.set_target(f.q);
        self.enabled.store(f.enabled, Ordering::Relaxed);
    }
}

struct CompRt {
    threshold: Smoothed,
    ratio: Smoothed,
    attack: Smoothed,
    release: Smoothed,
    makeup: Smoothed,
    enabled: AtomicBool,
}

impl CompRt {
    fn new(p: &CompressorParams) -> Self {
        Self {
            threshold: Smoothed::new(p.threshold_db, SAMPLE_RATE, PARAM_RAMP_MS),
            ratio: Smoothed::new(p.ratio, SAMPLE_RATE, PARAM_RAMP_MS),
            attack: Smoothed::new(p.attack_ms, SAMPLE_RATE, PARAM_RAMP_MS),
            release: Smoothed::new(p.release_ms, SAMPLE_RATE, PARAM_RAMP_MS),
            makeup: Smoothed::new(p.makeup_gain_db, SAMPLE_RATE, PARAM_RAMP_MS),
            enabled: AtomicBool::new(p.enabled),
        }
    }

    fn set(&self, p: &CompressorParams) {
        self.threshold.set_target(p.threshold_db);
        self.ratio.set_target(p.ratio);
        self.attack.set_target(p.attack_ms);
        self.release.set_target(p.release_ms);
        self.makeup.set_target(p.makeup_gain_db);
        self.enabled.store(p.enabled, Ordering::Relaxed);
    }

    #[inline]
    fn next(&self) -> CompressorParams {
        CompressorParams {
            threshold_db: self.threshold.next(),
            ratio: self.ratio.next(),
            attack_ms: self.attack.next(),
            release_ms: self.release.next(),
            makeup_gain_db: self.makeup.next(),
            enabled: self.enabled.load(Ordering::Relaxed),
        }
    }
}

struct GateRt {
    threshold: Smoothed,
    attack: Smoothed,
    release: Smoothed,
    hold: Smoothed,
    enabled: AtomicBool,
}

impl GateRt {
    fn new(p: &GateParams) -> Self {
        Self {
            threshold: Smoothed::new(p.threshold_db, SAMPLE_RATE, PARAM_RAMP_MS),
            attack: Smoothed::new(p.attack_ms, SAMPLE_RATE, PARAM_RAMP_MS),
            release: Smoothed::new(p.release_ms, SAMPLE_RATE, PARAM_RAMP_MS),
            hold: Smoothed::new(p.hold_ms, SAMPLE_RATE, PARAM_RAMP_MS),
            enabled: AtomicBool::new(p.enabled),
        }
    }

    fn set(&self, p: &GateParams) {
        self.threshold.set_target(p.threshold_db);
        self.attack.set_target(p.attack_ms);
        self.release.set_target(p.release_ms);
        self.hold.set_target(p.hold_ms);
        self.enabled.store(p.enabled, Ordering::Relaxed);
    }

    #[inline]
    fn next(&self) -> GateParams {
        GateParams {
            threshold_db: self.threshold.next(),
            attack_ms: self.attack.next(),
            release_ms: self.release.next(),
            hold_ms: self.hold.next(),
            enabled: self.enabled.load(Ordering::Relaxed),
        }
    }
}

/// Shared, lock-free live parameters for one group's DSP chain (PROJECT_SPEC
/// Section 6's processing order: gain -> EQ -> HPF/LPF -> compressor/gate).
struct GroupParams {
    gain: Smoothed,
    master_gain: Arc<Smoothed>,
    eq_bands: Vec<EqBandRt>,
    highpass: FilterRt,
    lowpass: FilterRt,
    compressor: CompRt,
    gate: GateRt,
}

impl GroupParams {
    fn new(group: &Group, master_gain: Arc<Smoothed>) -> Self {
        Self {
            gain: Smoothed::new(group.gain, SAMPLE_RATE, PARAM_RAMP_MS),
            master_gain,
            eq_bands: group.eq_bands.iter().map(EqBandRt::new).collect(),
            highpass: FilterRt::new(&group.highpass),
            lowpass: FilterRt::new(&group.lowpass),
            compressor: CompRt::new(&group.compressor),
            gate: GateRt::new(&group.gate),
        }
    }
}

/// A group's live PipeWire presence: a capture stream apps route into
/// (`media.class=Audio/Sink`) and a playback stream feeding the real output
/// device, bridged by a lock-free ring buffer so the two streams' callbacks
/// (which may run with different buffer sizes/timing) stay decoupled.
struct GroupNode {
    capture: pw::stream::StreamRc,
    // Only held to keep the stream alive — its process closure does the
    // actual work via the ring buffer, nothing here reads it back out.
    #[allow(dead_code)]
    playback: pw::stream::StreamRc,
    params: Arc<GroupParams>,
    _capture_listener: pw::stream::StreamListener<()>,
    _playback_listener: pw::stream::StreamListener<()>,
}

/// Enough of a Port global's properties to pair it up for linking.
struct PortInfo {
    node_id: u32,
    output: bool,
    /// "FL"/"FR"/etc, when the port advertises one.
    channel: Option<String>,
}

/// Enough of a Link global's properties to find and sever it later.
struct LinkInfo {
    output_node: u32,
    input_node: u32,
}

struct Engine {
    core: pw::core::CoreRc,
    registry: pw::registry::RegistryRc,
    master_gain: Arc<Smoothed>,
    groups: HashMap<u32, GroupNode>,
    ports: HashMap<u32, PortInfo>,
    links: HashMap<u32, LinkInfo>,
    /// Keeps each detected app stream's Node proxy + info listener alive so
    /// its `running` state (see `AppStream::running`) keeps updating. Pure
    /// bookkeeping — nothing here is read back, only dropped on removal.
    #[allow(dead_code)]
    stream_nodes: HashMap<u32, (pw::node::Node, pw::node::NodeListener)>,
    /// Name of the factory that creates Link objects, discovered from the
    /// registry the same way `create-delete-remote-objects.rs` does — the
    /// literal string is conventionally "link-factory" but isn't guaranteed.
    link_factory: Option<String>,
    /// Mirrors `groups`' keys so the D-Bus thread can read live meter data
    /// without touching anything pw-mainloop-thread-owned.
    live: Arc<Mutex<HashMap<u32, Arc<GroupLive>>>>,
    metrics: Arc<Metrics>,
    /// Same `Config` the D-Bus thread owns — shared so a newly-detected app
    /// stream can be auto-routed into the protected default group (see
    /// `auto_route_default`) and have that persisted the same way a manual
    /// `RouteStreamToGroup` call is.
    cfg: Arc<Mutex<Config>>,
    /// stream_id -> (group_id, attempts so far) for a route that couldn't
    /// complete yet because the stream's own ports (or, rarely, the target
    /// group's capture node) hadn't been announced by the registry as their
    /// own globals when the route was attempted — PipeWire fires a node's
    /// "global add" before its ports arrive as separate Port globals a
    /// moment later, and `auto_route_default` attempts the route the
    /// instant a stream is first seen, so it races this every single time.
    /// Retried from `on_global`'s Port branch, scoped to just the streams a
    /// given port could plausibly unblock (`retry_pending_routes_for_node`)
    /// — not every pending route on every port event anywhere in the graph,
    /// which both wastes the single mainloop thread's time budget on
    /// entirely unrelated streams and, worse, never gives up on a stream
    /// that will never reach a full stereo port set (some UI/notification
    /// sounds are genuinely mono), retrying it forever. `attempts` is what
    /// lets `route_stream` give up on requiring the full set after a few
    /// tries and link whatever's actually there instead.
    pending_routes: HashMap<u32, (u32, u32)>,
    /// stream_id -> group_id for every stream *currently* successfully
    /// routed — the persistent counterpart to `pending_routes`, which only
    /// tracks routes still being established. Some apps tear down and
    /// recreate a stream's ports (a format/corking renegotiation — observed
    /// on pause/resume) without destroying the node itself; without this,
    /// nothing ever notices the new ports need linking again, since
    /// `retry_pending_routes_for_node` only acted on streams still mid-setup
    /// — an already-routed stream whose ports got swapped out just sat
    /// silently unlinked. Checked by `retry_pending_routes_for_node`
    /// alongside `pending_routes`; kept in sync by `route_stream` (insert on
    /// success), `unroute_stream`, and stream removal.
    stream_group: HashMap<u32, u32>,
}

/// How many times `route_stream` insists on a full stereo port set on both
/// sides before accepting a partial (e.g. genuinely mono source) link
/// instead — bounds `pending_routes` retries so a stream that will never
/// grow a second port doesn't sit there forever getting re-tried.
const MAX_STRICT_ROUTE_ATTEMPTS: u32 = 5;

impl Engine {
    fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::CreateGroup(group) => self.create_group(&group),
            Command::DeleteGroup(id) => {
                self.groups.remove(&id);
                self.live.lock().unwrap().remove(&id);
            }
            Command::RouteStream { stream_id, group_id } => self.route_stream(stream_id, group_id),
            Command::UnrouteStream { stream_id } => self.unroute_stream(stream_id),
            Command::SetGain { group_id, value } => {
                if let Some(g) = self.groups.get(&group_id) {
                    g.params.gain.set_target(value);
                }
            }
            Command::SetMasterGain(value) => self.master_gain.set_target(value),
            Command::SetEqBand { group_id, index, band } => {
                if let Some(g) = self.groups.get(&group_id) {
                    if let Some(rt) = g.params.eq_bands.get(index) {
                        rt.set(&band);
                    }
                }
            }
            Command::SetHighpass { group_id, filter } => {
                if let Some(g) = self.groups.get(&group_id) {
                    g.params.highpass.set(&filter);
                }
            }
            Command::SetLowpass { group_id, filter } => {
                if let Some(g) = self.groups.get(&group_id) {
                    g.params.lowpass.set(&filter);
                }
            }
            Command::SetCompressor { group_id, params } => {
                if let Some(g) = self.groups.get(&group_id) {
                    g.params.compressor.set(&params);
                }
            }
            Command::SetGate { group_id, params } => {
                if let Some(g) = self.groups.get(&group_id) {
                    g.params.gate.set(&params);
                }
            }
        }
    }

    fn create_group(&mut self, group: &Group) {
        if self.groups.contains_key(&group.id) {
            return;
        }
        let live = Arc::new(GroupLive::default());
        self.live.lock().unwrap().insert(group.id, live.clone());
        match build_group_node(&self.core, group, self.master_gain.clone(), live, self.metrics.clone()) {
            Ok(node) => {
                tracing::info!(id = group.id, name = %group.name, "group created");
                self.groups.insert(group.id, node);
            }
            Err(err) => {
                tracing::error!(%err, id = group.id, "failed to create group");
                self.live.lock().unwrap().remove(&group.id);
            }
        }
    }

    /// An app stream auto-connects straight to the default hardware sink the
    /// moment it starts playing. Both routing it into a group and taking it
    /// back out must sever that (or any other) existing link first —
    /// otherwise two copies of the same signal can reach hardware at once,
    /// summed with a small relative delay (the group's ring buffer
    /// latency), which audibly phases/beats rather than just "having EQ
    /// applied". Also correctly handles re-routing an already-routed stream
    /// to a different group, since its old link gets torn down the same way.
    fn sever_existing_links(&self, stream_id: u32) {
        let stale_links: Vec<(u32, u32)> = self
            .links
            .iter()
            .filter(|(_, l)| l.output_node == stream_id)
            .map(|(id, l)| (*id, l.input_node))
            .collect();
        for (link_id, was_feeding) in stale_links {
            match self.registry.destroy_global(link_id).into_result() {
                Ok(_) => tracing::info!(stream_id, was_feeding, "severed stream's existing link"),
                Err(err) => tracing::warn!(%err, link_id, "failed to remove stream's existing link"),
            }
        }
    }

    /// Links every port of `stream_id` into the matching (channel-paired)
    /// port of `target_node`. Shared by routing into a group's capture node
    /// and by un-routing back to whatever hardware sink a group's own
    /// playback stream is already connected to. Returns whether linking was
    /// actually attempted — `false` means one side doesn't have enough
    /// ports announced yet (or no link factory), which the caller needs to
    /// know so it can retry rather than treat "nothing to link" as "done".
    /// `require_full_set`: wait for `CHANNELS` ports on both sides (avoids
    /// pairing a lone early port with the wrong channel via the
    /// `inputs.first()` fallback below — the "audio only in one channel"
    /// bug); once `route_stream` has retried enough times, this is `false`
    /// and just one port on each side is enough, so a genuinely mono source
    /// (some UI/notification sounds never grow a second port) still gets
    /// linked instead of retrying forever.
    fn link_stream_to_node(&self, stream_id: u32, target_node: u32, require_full_set: bool) -> bool {
        let Some(factory) = self.link_factory.clone() else {
            tracing::warn!("no link factory discovered yet, can't link");
            return false;
        };

        // Linking node-to-node (no explicit ports) makes PipeWire try to
        // create fresh ports for the link instead of reusing the existing
        // (format-negotiated, fixed) ones — observed failing server-side
        // with "no more port available". Pair up ports by channel
        // ourselves instead, the way `pw-link <src> <dst>` does.
        let mut outputs: Vec<_> =
            self.ports.iter().filter(|(_, p)| p.node_id == stream_id && p.output).collect();
        let mut inputs: Vec<_> =
            self.ports.iter().filter(|(_, p)| p.node_id == target_node && !p.output).collect();
        let min_ports = if require_full_set { CHANNELS } else { 1 };
        if outputs.len() < min_ports || inputs.len() < min_ports {
            tracing::warn!(stream_id, target_node, require_full_set, "stream or target doesn't have enough ports yet, try again shortly");
            return false;
        }
        // Deterministic order; channel-matched below when both sides label them.
        outputs.sort_by_key(|(id, _)| **id);
        inputs.sort_by_key(|(id, _)| **id);

        for (out_id, out_port) in &outputs {
            let matched_input = inputs
                .iter()
                .find(|(_, p)| out_port.channel.is_some() && p.channel == out_port.channel)
                .or_else(|| inputs.first())
                .map(|(id, _)| **id);
            let Some(in_id) = matched_input else { continue };

            let props = properties! {
                "link.output.port" => out_id.to_string(),
                "link.input.port" => in_id.to_string(),
                "link.output.node" => stream_id.to_string(),
                "link.input.node" => target_node.to_string(),
                // Keep the link alive on the server even though we drop our
                // local proxy to it immediately below.
                "object.linger" => "1",
            };
            match self.core.create_object::<pw::link::Link>(&factory, &props) {
                Ok(link) => std::mem::drop(link),
                Err(err) => tracing::error!(%err, out_id = **out_id, in_id, "failed to create link"),
            }
        }
        true
    }

    /// Routes `stream_id` into `group_id`. If either side's ports aren't
    /// announced yet, the attempt is recorded in `pending_routes` instead of
    /// silently dropped — `retry_pending_routes_for_node` retries it once
    /// real ports show up, so a route requested the instant a stream first
    /// appears (every `auto_route_default` call) doesn't leave that stream
    /// severed from hardware with nothing ever reconnecting it.
    fn route_stream(&mut self, stream_id: u32, group_id: u32) {
        let attempts = self.pending_routes.get(&stream_id).map_or(0, |&(_, a)| a);
        let Some(node) = self.groups.get(&group_id) else {
            tracing::warn!(group_id, "route requested for unknown group");
            return;
        };
        let input_node = node.capture.node_id();
        if input_node == u32::MAX {
            tracing::warn!(group_id, "group's capture node has no id yet, try again shortly");
            self.pending_routes.insert(stream_id, (group_id, attempts + 1));
            return;
        }
        self.sever_existing_links(stream_id);
        let require_full_set = attempts < MAX_STRICT_ROUTE_ATTEMPTS;
        if self.link_stream_to_node(stream_id, input_node, require_full_set) {
            self.pending_routes.remove(&stream_id);
            self.stream_group.insert(stream_id, group_id);
            tracing::info!(stream_id, group_id, "routed stream into group");
        } else {
            self.pending_routes.insert(stream_id, (group_id, attempts + 1));
        }
    }

    /// Retries every pending route that `node_id` could plausibly unblock —
    /// one whose own stream is `node_id`, or one whose target group's
    /// capture node is `node_id` — plus re-verifies `node_id` if it's a
    /// stream that's *already* routed (`stream_group`), in case this new
    /// port is a replacement for one that got torn down (a pause/resume
    /// renegotiation, observed live: the old port and its link quietly
    /// disappear via `global_remove`, and without this, the new one just
    /// sits unlinked — the video keeps playing, its audio doesn't). Called
    /// whenever a new Port global arrives for that node. Scoped rather than
    /// retrying every pending route on every port event anywhere in the
    /// graph: that blanket version wasted the single mainloop thread's time
    /// on streams a given port couldn't possibly affect, and turned a
    /// stream that will never reach a full stereo port set (some UI/
    /// notification sounds are genuinely mono) into a permanent storm —
    /// every other node's port churn kept re-triggering it. Re-severs on
    /// each attempt too: PipeWire's own autoconnect can have relinked the
    /// stream to hardware in the meantime, and leaving that link up
    /// alongside a newly-succeeded group link is exactly the double-link
    /// phasing bug `sever_existing_links` exists to prevent.
    fn retry_pending_routes_for_node(&mut self, node_id: u32) {
        let mut relevant: Vec<(u32, u32)> = self
            .pending_routes
            .iter()
            .filter(|(&stream_id, &(group_id, _))| {
                stream_id == node_id || self.groups.get(&group_id).is_some_and(|g| g.capture.node_id() == node_id)
            })
            .map(|(&stream_id, &(group_id, _))| (stream_id, group_id))
            .collect();
        if let Some(&group_id) = self.stream_group.get(&node_id) {
            relevant.push((node_id, group_id));
        }
        relevant.sort_unstable();
        relevant.dedup();
        for (stream_id, group_id) in relevant {
            self.route_stream(stream_id, group_id);
        }
    }

    /// Routes `stream_id` into `group_id` and persists it as that stream's
    /// one and only group in `Config`, stripping it from every other
    /// group's `routed_streams` first — the same one-group-only invariant
    /// `dbus.rs::route_stream_to_group` enforces for a manual route. Shared
    /// by `auto_route_default` and `on_global`'s "honor an existing
    /// assignment" path so both ways a stream can end up routed keep
    /// `Config` consistent; without the strip step here, a stream that
    /// raced auto-routing against a manual route (or a restart's rediscovery
    /// racing either) could end up listed under two groups'
    /// `routed_streams` at once — which is exactly the corrupted state a
    /// live config was found in.
    fn route_and_persist(&mut self, stream_id: u32, group_id: u32) {
        self.route_stream(stream_id, group_id);
        let mut cfg = self.cfg.lock().unwrap();
        for group in cfg.groups.iter_mut() {
            group.routed_streams.retain(|&id| id != stream_id);
        }
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            group.routed_streams.push(stream_id);
        }
        let _ = cfg.save();
    }

    /// A freshly-detected app stream that hasn't been explicitly routed
    /// anywhere always flows through the protected default group first —
    /// the "primary system for transmitting live audio" ask: nothing plays
    /// un-mixed by default, the user opts a stream *out* into its own group
    /// rather than opting every app *in*.
    ///
    /// Only called from `on_global` when `stream_id` isn't already listed
    /// under some group's `routed_streams` — see the caller. That check
    /// matters as much as anything in here: PipeWire replays every
    /// currently-live stream as a fresh "global add" on every daemon
    /// restart (same session, stable ids, nothing actually new), and
    /// calling this unconditionally for each one used to silently sweep
    /// every stream — including ones the user had deliberately routed
    /// elsewhere — back into Default on every restart.
    fn auto_route_default(&mut self, stream_id: u32) {
        let Some(group_id) = self.cfg.lock().unwrap().groups.iter().find(|g| g.protected).map(|g| g.id) else {
            return;
        };
        self.route_and_persist(stream_id, group_id);
    }

    /// Takes a stream back out of whichever group it's routed into and
    /// reconnects it to real hardware — "remove from group" means "back to
    /// normal playback", not "now silent". Finds "real hardware" by asking
    /// what any group's own (always-autoconnected) playback stream is
    /// currently linked to, rather than querying PipeWire's default-sink
    /// metadata directly, since that link already has to exist for any
    /// group a stream could have been routed into in the first place.
    fn unroute_stream(&mut self, stream_id: u32) {
        self.stream_group.remove(&stream_id);
        self.sever_existing_links(stream_id);
        let hw_node = self.groups.values().find_map(|g| {
            let playback_node = g.playback.node_id();
            self.links.values().find(|l| l.output_node == playback_node).map(|l| l.input_node)
        });
        match hw_node {
            Some(hw_node) => {
                self.link_stream_to_node(stream_id, hw_node, true);
                tracing::info!(stream_id, hw_node, "unrouted stream back to hardware");
            }
            None => tracing::warn!(stream_id, "unrouted stream, but no hardware sink found to reconnect it to"),
        }
    }
}

fn audio_format_params() -> Vec<u8> {
    let mut audio_info = pw::spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(pw::spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(SAMPLE_RATE as u32);
    audio_info.set_channels(CHANNELS as u32);

    let obj = Object {
        type_: SpaTypes::ObjectParamFormat.as_raw(),
        id: pw::spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &Value::Object(obj))
        .expect("audio format pod always serializes")
        .0
        .into_inner()
}

fn build_group_node(
    core: &pw::core::CoreRc,
    group: &Group,
    master_gain: Arc<Smoothed>,
    live: Arc<GroupLive>,
    metrics: Arc<Metrics>,
) -> Result<GroupNode, pw::Error> {
    let params = Arc::new(GroupParams::new(group, master_gain));

    let capture = pw::stream::StreamRc::new(
        core.clone(),
        &format!("mtxpanel.group.{}", group.id),
        properties! {
            *keys::MEDIA_TYPE => "Audio",
            *keys::MEDIA_CLASS => "Audio/Sink",
            *keys::NODE_NAME => format!("mtxpanel.group.{}", group.id),
            *keys::NODE_DESCRIPTION => group.name.as_str(),
        },
    )?;

    let (mut ring_tx, mut ring_rx) = rtrb::RingBuffer::<f32>::new(RING_CAPACITY);

    // Per-band/per-channel filter state lives only in this closure's capture
    // — it's mutated every sample but never touched from another thread.
    let band_count = params.eq_bands.len();
    let mut eq_state: Vec<[Biquad; CHANNELS]> = vec![[Biquad::default(); CHANNELS]; band_count];
    let mut hp_state = [Biquad::default(); CHANNELS];
    let mut lp_state = [Biquad::default(); CHANNELS];
    let mut dynamics = DynamicsProcessor::new();
    // recompute_every=4: an FFT every callback (~10ms) is wasted work no
    // 200ms-polled UI could ever show; ~40ms cadence still reads as live.
    let mut spectrum = SpectrumAnalyzer::new(SAMPLE_RATE, 4);

    let capture_params = params.clone();
    let capture_metrics = metrics.clone();
    let capture_live = live.clone();
    let capture_listener = capture
        .add_local_listener_with_user_data(())
        .process(move |stream, _| {
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }
            capture_live.mark_active();
            let data = &mut datas[0];
            let stride = CHANNELS * std::mem::size_of::<f32>();
            let n_frames = data.chunk().size() as usize / stride;
            capture_metrics.buffer_frames.store(n_frames as u32, Ordering::Relaxed);
            let Some(slice) = data.data() else { return };

            // Rebuild filter coefficients once per buffer (not per sample —
            // trig is comparatively expensive) from whatever the smoothers
            // currently hold; per-sample `.next()` below keeps them ramping
            // regardless of how often we rebuild from them.
            for (band, state) in capture_params.eq_bands.iter().zip(eq_state.iter_mut()) {
                if band.enabled.load(Ordering::Relaxed) {
                    let snapshot = band.snapshot_current();
                    for ch in state.iter_mut() {
                        ch.set_peaking(&snapshot, SAMPLE_RATE);
                    }
                }
            }
            if capture_params.highpass.enabled.load(Ordering::Relaxed) {
                let (freq, q) = (capture_params.highpass.freq.current(), capture_params.highpass.q.current());
                for ch in hp_state.iter_mut() {
                    ch.set_highpass(freq, q, SAMPLE_RATE);
                }
            }
            if capture_params.lowpass.enabled.load(Ordering::Relaxed) {
                let (freq, q) = (capture_params.lowpass.freq.current(), capture_params.lowpass.q.current());
                for ch in lp_state.iter_mut() {
                    ch.set_lowpass(freq, q, SAMPLE_RATE);
                }
            }

            let mut buf_peak = [0f32; CHANNELS];
            for frame in 0..n_frames {
                // ponytail: every group's closure ticks the shared master
                // ramp once per frame, so with N groups active it converges
                // ~N times faster than PARAM_RAMP_MS states — harmless (still
                // smooth, just brisker) rather than a race, since everything
                // here runs on the one pw mainloop thread. Fix path: apply
                // master gain once at a single shared mix point instead of
                // once per group, if that speed-up becomes audible.
                let gain = capture_params.gain.next() * capture_params.master_gain.next();
                let comp = capture_params.compressor.next();
                let gate = capture_params.gate.next();
                // Keep every band's smoother ticking each sample even though
                // coefficients only get rebuilt once per buffer above.
                for band in &capture_params.eq_bands {
                    band.freq.next();
                    band.gain_db.next();
                    band.q.next();
                }
                capture_params.highpass.freq.next();
                capture_params.highpass.q.next();
                capture_params.lowpass.freq.next();
                capture_params.lowpass.q.next();

                let mut samples = [0f32; CHANNELS];
                for (ch, sample) in samples.iter_mut().enumerate() {
                    let start = frame * stride + ch * 4;
                    let bytes: [u8; 4] = slice[start..start + 4].try_into().unwrap();
                    let raw = f32::from_le_bytes(bytes);
                    // Analyze the *input* (pre gain/EQ), left channel only
                    // — "show me what's arriving, not what I already did
                    // to it" (Section 7.1/7.3's spectrum ask), and one
                    // channel is plenty for a visual analyzer.
                    if ch == 0 {
                        spectrum.push(raw);
                    }
                    let mut x = raw * gain;

                    if capture_params.highpass.enabled.load(Ordering::Relaxed) {
                        x = hp_state[ch].process(x);
                    }
                    if capture_params.lowpass.enabled.load(Ordering::Relaxed) {
                        x = lp_state[ch].process(x);
                    }
                    for (band, state) in capture_params.eq_bands.iter().zip(eq_state.iter_mut()) {
                        if band.enabled.load(Ordering::Relaxed) {
                            x = state[ch].process(x);
                        }
                    }
                    *sample = x;
                }

                let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
                let mult = {
                    let out = dynamics.process(peak.max(1e-9), &comp, &gate, SAMPLE_RATE);
                    out / peak.max(1e-9)
                };

                for (ch, sample) in samples.into_iter().enumerate() {
                    let out = sample * mult;
                    buf_peak[ch] = buf_peak[ch].max(out.abs());
                    let _ = ring_tx.push(out);
                }
            }
            capture_live.publish(buf_peak[0], buf_peak[1], dynamics.gain_reduction_db(), dynamics.gate_reduction_db());
            if let Some(bins) = spectrum.maybe_compute() {
                capture_live.publish_spectrum(&bins);
            }
        })
        .register()?;

    let format_bytes = audio_format_params();
    let mut params_vec = [Pod::from_bytes(&format_bytes).unwrap()];
    capture.connect(
        Direction::Input,
        None,
        pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS,
        &mut params_vec,
    )?;

    let playback = pw::stream::StreamRc::new(
        core.clone(),
        &format!("mtxpanel.group.{}.out", group.id),
        properties! {
            *keys::MEDIA_TYPE => "Audio",
            *keys::MEDIA_CATEGORY => "Playback",
            *keys::MEDIA_ROLE => "Music",
            *keys::NODE_NAME => format!("mtxpanel.group.{}.out", group.id),
            *keys::NODE_PASSIVE => "true",
        },
    )?;

    let playback_metrics = metrics.clone();
    let playback_live = live.clone();
    let playback_listener = playback
        .add_local_listener_with_user_data(())
        .process(move |stream, _| {
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            // `requested` is how many frames the graph actually wants
            // produced *this cycle* — not the raw buffer's full capacity.
            // Using `slice.len()` (the old code) instead meant popping way
            // more samples per callback than the capture side had time to
            // push in the same cycle whenever the negotiated buffer was
            // larger than one quantum, silently draining the ring buffer
            // far faster than it filled. That's the looping/cutting-out
            // bug: not a metric artifact, an actual sample-count mismatch
            // between how much this closure produced and how much the
            // capture closure had actually supplied. `requested` can be 0
            // when the driver hasn't reported it yet; fall back to the
            // buffer's own capacity only in that case, same as before.
            let requested = buffer.requested() as usize;
            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }
            let data = &mut datas[0];
            let stride = CHANNELS * std::mem::size_of::<f32>();
            let Some(slice) = data.data() else { return };
            let capacity_frames = slice.len() / stride;
            let n_frames = if requested > 0 { requested.min(capacity_frames) } else { capacity_frames };

            for frame in 0..n_frames {
                for ch in 0..CHANNELS {
                    let sample = ring_rx.pop().unwrap_or_else(|_| {
                        // Only counts once this group has ever actually
                        // received app audio — see `GroupLive::has_input`'s
                        // doc comment for why.
                        if playback_live.has_input() {
                            playback_metrics.underruns.fetch_add(1, Ordering::Relaxed);
                        }
                        0.0
                    });
                    let start = frame * stride + ch * 4;
                    slice[start..start + 4].copy_from_slice(&sample.to_le_bytes());
                }
            }
            let chunk = data.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = stride as _;
            *chunk.size_mut() = (stride * n_frames) as _;
        })
        .register()?;

    let format_bytes = audio_format_params();
    let mut params_vec = [Pod::from_bytes(&format_bytes).unwrap()];
    playback.connect(
        Direction::Output,
        None,
        pw::stream::StreamFlags::AUTOCONNECT
            | pw::stream::StreamFlags::MAP_BUFFERS
            | pw::stream::StreamFlags::RT_PROCESS,
        &mut params_vec,
    )?;

    Ok(GroupNode {
        capture,
        playback,
        params,
        _capture_listener: capture_listener,
        _playback_listener: playback_listener,
    })
}

pub fn run(
    cfg: Config,
    cmd_rx: pw::channel::Receiver<Command>,
    streams: Arc<Mutex<HashMap<u32, AppStream>>>,
    live: Arc<Mutex<HashMap<u32, Arc<GroupLive>>>>,
    metrics: Arc<Metrics>,
    cfg_state: Arc<Mutex<Config>>,
) -> Result<(), pw::Error> {
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;

    let engine = Rc::new(RefCell::new(Engine {
        core: core.clone(),
        registry: registry.clone(),
        master_gain: Arc::new(Smoothed::new(cfg.master_gain, SAMPLE_RATE, PARAM_RAMP_MS)),
        groups: HashMap::new(),
        ports: HashMap::new(),
        links: HashMap::new(),
        stream_nodes: HashMap::new(),
        link_factory: None,
        live,
        metrics,
        cfg: cfg_state,
        pending_routes: HashMap::new(),
        stream_group: HashMap::new(),
    }));

    for group in &cfg.groups {
        engine.borrow_mut().create_group(group);
    }

    let streams_for_registry = streams.clone();
    let engine_for_registry = engine.clone();
    let engine_for_remove = engine.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| on_global(global, &streams_for_registry, &engine_for_registry))
        .global_remove(move |id| {
            streams.lock().unwrap().remove(&id);
            let mut engine = engine_for_remove.borrow_mut();
            engine.ports.remove(&id);
            engine.links.remove(&id);
            engine.stream_nodes.remove(&id);
            // A stream that closed before its route ever completed (tab
            // shut fast) would otherwise sit in here forever, retried
            // pointlessly on every later port event.
            engine.pending_routes.remove(&id);
            engine.stream_group.remove(&id);
        })
        .register();

    let engine_for_cmd = engine.clone();
    let _cmd_receiver = cmd_rx.attach(mainloop.loop_(), move |cmd| {
        engine_for_cmd.borrow_mut().handle_command(cmd);
    });

    tracing::info!("connected to pipewire, watching for app streams (ctrl-c to quit)");
    mainloop.run();

    Ok(())
}

fn on_global(
    global: &GlobalObject<&DictRef>,
    streams: &Arc<Mutex<HashMap<u32, AppStream>>>,
    engine: &Rc<RefCell<Engine>>,
) {
    let Some(props) = global.props else { return };

    if props.get("factory.type.name") == Some(pw::types::ObjectType::Link.to_str()) {
        if let Some(name) = props.get("factory.name") {
            engine.borrow_mut().link_factory = Some(name.to_owned());
        }
        return;
    }

    if global.type_ == pw::types::ObjectType::Port {
        let Some(node_id) = props.get("node.id").and_then(|s| s.parse().ok()) else { return };
        let output = props.get("port.direction") == Some("out");
        let channel = props.get("audio.channel").map(str::to_owned);
        let mut engine = engine.borrow_mut();
        engine.ports.insert(global.id, PortInfo { node_id, output, channel });
        // A port just showed up on `node_id` — retry only the pending
        // routes it could actually unblock, not every pending route in the
        // graph (see `retry_pending_routes_for_node`'s doc comment).
        engine.retry_pending_routes_for_node(node_id);
        return;
    }

    if global.type_ == pw::types::ObjectType::Link {
        let Some(output_node) = props.get("link.output.node").and_then(|s| s.parse().ok()) else { return };
        let Some(input_node) = props.get("link.input.node").and_then(|s| s.parse().ok()) else { return };
        engine.borrow_mut().links.insert(global.id, LinkInfo { output_node, input_node });
        return;
    }

    let Some(media_class) = props.get("media.class") else { return };
    if media_class != "Stream/Output/Audio" {
        return;
    }
    let Some(node_name) = props.get("node.name") else { return };
    if node_name.starts_with("mtxpanel.") {
        return; // our own group nodes, not routable app streams
    }

    let name = props
        .get("application.name")
        .unwrap_or(node_name)
        .to_owned();
    let service = props.get("media.name").and_then(extract_service_name).filter(|s| s != &name).unwrap_or_default();
    tracing::info!(id = global.id, name, service, "app stream");
    streams.lock().unwrap().insert(global.id, AppStream { id: global.id, name, service, running: false });
    // A daemon restart replays every currently-live stream as a fresh
    // "global add" — same PipeWire session, same still-valid ids, nothing
    // actually new. If this id is already listed under some group from a
    // prior load of Config, that's a real (if stale-looking) manual
    // assignment the user made, not something to discard: honor it instead
    // of sweeping it back into Default (see `auto_route_default`'s doc
    // comment for what used to happen here).
    let existing_group =
        engine.borrow().cfg.lock().unwrap().groups.iter().find(|g| g.routed_streams.contains(&global.id)).map(|g| g.id);
    match existing_group {
        // route_and_persist (not the bare route_stream) even though this
        // group already "has" the id — it also strips stream_id from every
        // *other* group's list, which cleans up a stale duplicate entry
        // left over from before this whole check existed.
        Some(group_id) => engine.borrow_mut().route_and_persist(global.id, group_id),
        None => engine.borrow_mut().auto_route_default(global.id),
    }

    // A name alone doesn't tell "is this one actually making sound right
    // now" apart from another instance of the same app (Section 7.2, and
    // `service` above can still collide — two tabs on the same site). Bind
    // the node and track its RUNNING state rather than guessing from a name.
    let stream_id = global.id;
    let registry = engine.borrow().registry.clone();
    match registry.bind::<pw::node::Node, _>(global) {
        Ok(node) => {
            let streams_for_info = streams.clone();
            let listener = node
                .add_listener_local()
                .info(move |info| {
                    let running = matches!(info.state(), pw::node::NodeState::Running);
                    // `media.name` (a browser tab's title, most often) isn't
                    // set yet in the registry's initial global-add snapshot
                    // for a just-created stream — it, and any later change
                    // (new video, tab navigated), arrives through this same
                    // info listener instead, not a fresh registry event. Only
                    // this callback ever sees it live.
                    let service = info
                        .props()
                        .and_then(|p| p.get("media.name"))
                        .and_then(extract_service_name);
                    if let Some(s) = streams_for_info.lock().unwrap().get_mut(&stream_id) {
                        s.running = running;
                        if let Some(service) = service.filter(|svc| svc != &s.name) {
                            s.service = service;
                        }
                    }
                })
                .register();
            engine.borrow_mut().stream_nodes.insert(stream_id, (node, listener));
        }
        Err(err) => tracing::warn!(%err, id = stream_id, "failed to bind stream node for running-state tracking"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_and_group_live_round_trip_through_atomics() {
        let m = Metrics::default();
        assert_eq!(m.cpu_pct(), 0.0);
        m.set_cpu_pct(42.5);
        m.set_ram_pct(80.0);
        assert_eq!(m.cpu_pct(), 42.5);
        assert_eq!(m.ram_pct(), 80.0);

        let live = GroupLive::default();
        assert_eq!(live.read(), (0.0, 0.0, 0.0, 0.0));
        live.publish(0.5, 0.75, -6.0, -3.0);
        assert_eq!(live.read(), (0.5, 0.75, -6.0, -3.0));

        assert_eq!(live.read_spectrum(), [crate::dsp::spectrum::FLOOR_DB; crate::dsp::spectrum::N_BINS]);
        let mut bins = [0.0; crate::dsp::spectrum::N_BINS];
        bins[3] = -12.5;
        live.publish_spectrum(&bins);
        assert_eq!(live.read_spectrum(), bins);
    }

    #[test]
    fn extract_service_name_cases() {
        assert_eq!(
            extract_service_name("(1) Benson Boone - Beautiful Things (Rock Cover by Our Last Night) - YouTube"),
            Some("YouTube".to_owned())
        );
        assert_eq!(extract_service_name("(57) WhatsApp"), Some("WhatsApp".to_owned()));
        assert_eq!(extract_service_name("Spotify"), Some("Spotify".to_owned()));
        assert_eq!(extract_service_name("Song Title - Spotify"), Some("Spotify".to_owned()));
        assert_eq!(extract_service_name(""), None);
    }
}
