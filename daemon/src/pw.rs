//! PipeWire engine: connects to PipeWire, tracks per-app audio streams, and
//! owns one capture+playback stream pair per group (PROJECT_SPEC.md Section
//! 4/6, build-order stages 1-3/5/6), plus (NewImprovenments/
//! MTX_AUDIO_GROUPS_MULTI_OUTPUT.md / MTX_MULTI_OUTPUT_DELAY_COMPENSATION.md)
//! one playback stream per group *output route* when a group has explicit
//! output routing configured.
//!
//! **Threading model — corrected after a live crash, read this before
//! sharing new state with a `process()` closure.** Registry events and
//! `Command` handling run on the single thread that calls `run()`
//! (pipewire-rs's `pw_main_loop` wrapper dispatches both there). Every
//! stream connected with `StreamFlags::RT_PROCESS` (every capture and
//! playback stream this file creates) has its `process()` callback invoked
//! from **PipeWire's own realtime thread** instead — this is
//! `PW_STREAM_FLAG_RT_PROCESS`'s documented, literal meaning ("call process
//! from the realtime thread", `pipewire/stream.h`), not a same-thread
//! dispatch through the mainloop. An earlier version of this comment
//! claimed the opposite ("everything...runs on the single thread") and
//! that led directly to a real production bug: state shared between
//! `Engine` (control thread) and a `process()` closure (RT thread) via
//! plain `Rc<RefCell<_>>` is unsound, not just occasionally slow — verified
//! live as a genuine `already borrowed` panic under real audio load, not a
//! hypothetical. Anything a `process()` closure and `Engine` both touch
//! (`RouteNode::delay`, `GroupNode::{route_producers,plugin_chain,
//! recording_producers}`) uses `Arc<Mutex<_>>` instead — see
//! `RouteDelay`'s doc comment for why a brief, infrequent lock here is an
//! acceptable real-time-safety tradeoff rather than a hazard. Per-sample
//! *parameter* values (gain, EQ coefficients, etc.) still use the
//! lock-free `Smoothed`/`AtomicBool` atomics `dsp::smoothed` already
//! provides — those remain correct and preferred for anything read every
//! sample; `Arc<Mutex<_>>` here is specifically for the handful of
//! *structural* collections (route/plugin/recording lists) that change
//! only on infrequent control events, matching this file's original
//! "ponytail" reasoning about accepting a small control-op stall — just
//! implemented with a real lock now instead of an unsound cell.
//!
//! `Engine` itself (`Rc<RefCell<Engine>>`) is unaffected by any of this: it
//! is only ever touched from the control thread (registry callbacks,
//! `Command` handling, the SIGTERM/SIGINT handlers, the ducking timer) —
//! never from inside a `process()` closure — so that one genuinely has no
//! cross-thread access and needs no lock.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pipewire as pw;
use pw::keys;
use pw::properties::properties;
use pw::registry::GlobalObject;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Object, Pod, Value};
use pw::spa::utils::dict::DictRef;
use pw::spa::utils::{Direction, SpaTypes};

use crate::config::Config;
use crate::ducking;
use crate::dsp::biquad::{Biquad, EqBand, FilterSlope};
use crate::dsp::delay::DelayLine;
use crate::dsp::dynamics::{CompressorParams, DynamicsProcessor, GateParams};
use crate::dsp::smoothed::Smoothed;
use crate::dsp::spectrum::SpectrumAnalyzer;
use crate::group::{Group, GroupOutputRoute};
use crate::identity::{self, AppId, OutputId};
use crate::output::{self, OutputDevice, RouteMode};
use crate::plugins;
use crate::rules;
use crate::scenes::SceneActivationReport;

const SAMPLE_RATE: f32 = 48_000.0;
const CHANNELS: usize = 2;
const PARAM_RAMP_MS: f32 = 15.0;
/// ~170ms of stereo headroom at 48kHz between the capture and playback
/// callbacks, which may run with different buffer sizes/timing.
const RING_CAPACITY: usize = 1 << 14;
/// Hard ceiling on how much delay any route's delay line can ever be asked
/// to hold, regardless of the user-configured "maximum compensation" — sized
/// once at route creation so raising the configured max later never needs to
/// reallocate a route's buffer (MTX_MULTI_OUTPUT_DELAY_COMPENSATION.md §21's
/// "avoid unnecessary allocations" in the processing path).
pub(crate) const ROUTE_DELAY_BUFFER_MS: f32 = 500.0;

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
    /// Stable identity (`identity::app_identity`) — what Scenes/Profiles/
    /// Rules key on instead of `id`, which is only valid for this stream's
    /// current lifetime.
    pub app_id: AppId,
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
/// Also the underrun gate: a group's *playback* stream(s) autoconnect to
/// real hardware, so PipeWire's graph driver calls their `process()` every
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
    /// Adds or updates one of this group's output routes (identity.rs's
    /// `OutputId`, not a transient node id) and re-syncs its live PipeWire
    /// state. `Config` is updated by the D-Bus thread before this is sent —
    /// this just tells the engine to catch up.
    SyncGroupRoutes { group_id: u32 },
    SetMaxCompensationMs(f32),
    ActivateScene { scene_id: u32 },
    /// Manual "Load Profile" (`profiles.rs`) — re-applies a saved profile
    /// to a stream that's already live, distinct from the automatic apply
    /// `on_global` already does the moment a matching stream first appears.
    ApplyProfile { stream_id: u32, app_id: AppId },
    /// Rebuilds a group's live plugin chain from its persisted
    /// `Group.plugins` list (`plugins.rs`) — same "tell the engine to
    /// catch up after `Config` already changed" shape as `SyncGroupRoutes`.
    SyncGroupPlugins { group_id: u32 },
    StartRecording { id: u32, group_id: u32, path: std::path::PathBuf, format: crate::recording::RecordingFormat },
    StopRecording { id: u32 },
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
    /// Ducking's own multiplier (`ducking.rs`), independent of the user's
    /// persisted `gain` fader — set from `Engine::evaluate_ducking`'s ~20ms
    /// tick, smoothed here (its intended job: interpolate between ticks
    /// without a zipper step, not shape attack/hold/release — that envelope
    /// is already computed by `DuckingEngine` before it ever reaches this).
    duck_gain: Smoothed,
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
            duck_gain: Smoothed::new(1.0, SAMPLE_RATE, PARAM_RAMP_MS),
            eq_bands: group.eq_bands.iter().map(EqBandRt::new).collect(),
            highpass: FilterRt::new(&group.highpass),
            lowpass: FilterRt::new(&group.lowpass),
            compressor: CompRt::new(&group.compressor),
            gate: GateRt::new(&group.gate),
        }
    }
}

/// A route's key into a group's live route table: `None` is the implicit
/// default route (today's bare-AUTOCONNECT behavior, used whenever a group
/// has no explicit `output_routes` configured); `Some(id)` is an explicit
/// route to that stable output.
type RouteKey = Option<OutputId>;

/// One end of the fan-out from a group's shared post-DSP signal to each of
/// its live routes' playback streams — the capture callback pushes the same
/// processed sample into every entry here once per frame.
struct RouteProducer {
    key: RouteKey,
    producer: rtrb::Producer<f32>,
}

/// Per-channel delay lines for one route (`dsp::delay::DelayLine`), boxed
/// behind an `Arc<Mutex<_>>` shared between the route's own `process()`
/// closure (which advances them every sample — running on PipeWire's own
/// realtime thread, per `PW_STREAM_FLAG_RT_PROCESS`'s documented behavior,
/// genuinely *not* the same thread `Engine` runs on) and `Engine` (which
/// updates the target whenever sync settings change, from the control
/// thread). `Rc<RefCell<_>>` was used here originally on the mistaken
/// assumption — this file's own now-corrected module doc previously made
/// the same claim — that everything in this file shares one thread;
/// confirmed live (a real, reproducible crash) that RT_PROCESS streams do
/// not. The lock is only ever briefly held (a handful of field writes on
/// an explicit user action, or one array index + interpolate per sample),
/// never across a blocking operation, so contention is not a real-time
/// hazard in practice — the same "brief, infrequent control-thread
/// contention is acceptable" tradeoff this file's `ponytail` comment on
/// the single-mainloop-thread design already documents elsewhere.
struct RouteDelay {
    lines: [DelayLine; CHANNELS],
}

impl RouteDelay {
    fn new(max_delay_samples: usize) -> Self {
        Self { lines: std::array::from_fn(|_| DelayLine::new(max_delay_samples)) }
    }

    fn set_delay_ms(&mut self, ms: f32, sample_rate: f32) {
        for line in &mut self.lines {
            line.set_delay_ms(ms, sample_rate);
        }
    }
}

/// A group's live route to one output (or the implicit default — see
/// `RouteKey`): its own playback stream, gain, enable flag, and delay line,
/// all independent of every other route the group has.
struct RouteNode {
    #[allow(dead_code)]
    playback: pw::stream::StreamRc,
    gain: Arc<Smoothed>,
    enabled: Arc<AtomicBool>,
    delay: Arc<Mutex<RouteDelay>>,
    _listener: pw::stream::StreamListener<()>,
}

/// A group's live PipeWire presence: a capture stream apps route into
/// (`media.class=Audio/Sink`), plus one or more routes bridging it (via a
/// per-route lock-free ring buffer) to real output devices.
struct GroupNode {
    capture: pw::stream::StreamRc,
    params: Arc<GroupParams>,
    route_producers: Arc<Mutex<Vec<RouteProducer>>>,
    routes: HashMap<RouteKey, RouteNode>,
    /// Rebuilt wholesale by `Engine::set_group_plugins` whenever the
    /// persisted plugin list changes (`plugins.rs`'s own doc comment
    /// explains why a full rebuild, not in-place mutation, is fine here).
    plugin_chain: Arc<Mutex<plugins::PluginChain>>,
    /// Active recording taps for this group (`recording.rs`) — fed the
    /// exact same post-DSP-and-plugins signal each output route gets, in
    /// the same per-sample fan-out loop.
    recording_producers: Arc<Mutex<Vec<RecordingProducer>>>,
    _capture_listener: pw::stream::StreamListener<()>,
}

struct RecordingProducer {
    id: u32,
    producer: rtrb::Producer<f32>,
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

/// Snapshots every key/value pair of a live registry `DictRef` into an owned
/// map — used to remember a Device global's or a Sink Node's properties
/// past the single callback that announces them (a `DictRef` only borrows
/// for that callback's duration). Whole-dict rather than an allowlist: both
/// `identity::output_identity`/`output_kind`/`estimate_output_latency` and
/// `OutputDevice`'s own fields each read a different handful of keys, and
/// keeping every key here means adding a new one of those reads later never
/// needs a matching change to what gets cached.
fn snapshot_props(props: &DictRef) -> HashMap<String, String> {
    props.iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect()
}

/// (Re-)classifies and registers the output backed by Sink/Duplex Node
/// `node_id`, using whatever Device data is cached for it right now. Called
/// once when the Node itself is first seen, and again from the Device
/// branch above if that Device data was still missing at the time (see its
/// comment) — `add_or_update_output` overwrites the previous entry either
/// way, so a second call just replaces a worse-guessed `OutputKind`/id.
fn discover_output_from_node(engine: &mut Engine, node_id: u32) {
    let Some(node_props) = engine.sink_nodes.get(&node_id).cloned() else { return };
    let device_props = node_props.get("device.id").and_then(|s| s.parse::<u32>().ok()).and_then(|id| engine.devices.get(&id));

    let node_name = node_props.get("node.name").map(String::as_str).unwrap_or("unknown-output");
    let id = identity::output_identity(&node_props, device_props);
    let kind = identity::output_kind(&node_props, device_props);
    let is_system_default = engine.system_default_sink.as_deref() == Some(id.as_str());
    let latency = identity::estimate_output_latency(&node_props);
    let description = node_props.get("node.description").map(String::as_str).unwrap_or(node_name).to_owned();
    let output = OutputDevice {
        id: id.clone(),
        name: node_name.to_owned(),
        description,
        kind,
        node_id: Some(node_id),
        is_system_default,
        sample_rate: node_props.get("audio.rate").and_then(|s| s.parse().ok()).unwrap_or(SAMPLE_RATE as u32),
        channels: node_props.get("audio.channels").and_then(|s| s.parse().ok()).unwrap_or(CHANNELS as u32),
        latency,
    };
    tracing::info!(id = %output.id, name = output.name, kind = output.kind.as_str(), "output discovered");
    engine.add_or_update_output(output);
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
    /// Discovered PipeWire outputs (MTX_AUDIO_GROUPS_MULTI_OUTPUT.md §4/§11),
    /// keyed by stable `OutputId` — mirrored to the D-Bus thread the same
    /// way `live`/`streams` are.
    outputs: Arc<Mutex<HashMap<OutputId, OutputDevice>>>,
    /// Device global id -> its cached `info.props` (bus/form-factor/serial/
    /// bluez address/etc.), populated once that Device's `info` event
    /// arrives — not from the registry global-add event itself, whose own
    /// advertised properties are minimal and don't include these (confirmed
    /// live: `device.bus` etc. came back missing until this was bound).
    devices: HashMap<u32, HashMap<String, String>>,
    /// Keeps each Device global's proxy + info listener alive so the above
    /// keeps updating — same bookkeeping-only role `stream_nodes` plays for
    /// app streams' `running` state.
    #[allow(dead_code)]
    device_proxies: HashMap<u32, (pw::device::Device, pw::device::DeviceListener)>,
    /// Sink/Duplex Node global id -> its cached registry properties, kept
    /// around (not just used once) so a Device's `info` event that arrives
    /// *after* its child Sink node's own global-add — real, observed
    /// PipeWire behavior, not hypothetical, and doubly likely here since the
    /// Device side now needs its own extra bind+info round trip — can
    /// trigger re-classifying that output once the device data it needed
    /// actually shows up. See `on_global`'s Device branch.
    sink_nodes: HashMap<u32, HashMap<String, String>>,
    /// output_id -> group ids whose wanted route to it couldn't be linked
    /// yet because the device wasn't live — same shape/purpose as
    /// `pending_routes`, just for output devices instead of app streams.
    /// Retried from `on_global`'s Sink-node-arrival branch.
    pending_output_routes: HashMap<OutputId, Vec<u32>>,
    /// The output PipeWire's own "default" metadata object currently names
    /// (§5's final fallback rung) — `None` until that metadata object shows
    /// up and reports one.
    system_default_sink: Option<OutputId>,
    /// User-configurable ceiling on automatic/manual delay compensation
    /// (MTX_MULTI_OUTPUT_DELAY_COMPENSATION.md §12), independent of
    /// `ROUTE_DELAY_BUFFER_MS`'s fixed allocation size.
    max_compensation_ms: f32,
    /// Same map the D-Bus thread reads from — `Engine` needs it too now:
    /// scene activation and rule actions resolve an `AppId` to whichever
    /// live stream currently has it (§0.2/Scenes/Rules).
    streams: Arc<Mutex<HashMap<u32, AppStream>>>,
    rule_runtime: rules::RuleRuntime,
    /// app_id -> group_id for a scene assignment that couldn't be applied
    /// immediately at activation time because that app wasn't running yet
    /// (NewImprovenments/01-audio-scenes.md's "Recovery" section) — checked
    /// by `on_global` the moment a matching app stream actually appears.
    active_scene_pending: HashMap<AppId, u32>,
    duck_engine: ducking::DuckingEngine,
    /// Active recordings (`recording.rs`), keyed by their D-Bus-assigned id
    /// — owned here (mainloop thread only), but each entry's status is also
    /// lock-freely readable via `recording_status` below.
    recordings: HashMap<u32, crate::recording::RecordingSession>,
    /// Mirror of just the pieces the D-Bus thread needs for a status query
    /// (elapsed time, frames written) — every field here is either
    /// immutable after insertion or a lock-free atomic/`Instant`, so no
    /// "refresh on change" bookkeeping is needed: a read is always current.
    recording_status: Arc<Mutex<HashMap<u32, RecordingStatus>>>,
}

/// What the D-Bus thread can read about an active recording without
/// touching `Engine` (which only the PipeWire mainloop thread owns).
#[derive(Clone)]
pub struct RecordingStatus {
    pub group_id: u32,
    pub path: String,
    pub started_at: std::time::Instant,
    pub frames_written_bits: Arc<AtomicU64>,
}

impl RecordingStatus {
    pub fn elapsed_secs(&self) -> f32 {
        self.started_at.elapsed().as_secs_f32()
    }
    pub fn frames_written(&self) -> u64 {
        self.frames_written_bits.load(Ordering::Relaxed)
    }
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
                self.finalize_recordings_for_group(id);
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
            Command::SyncGroupRoutes { group_id } => self.sync_group_routes(group_id),
            Command::SetMaxCompensationMs(ms) => {
                self.max_compensation_ms = ms.max(0.0);
                let group_ids: Vec<u32> = self.groups.keys().copied().collect();
                for id in group_ids {
                    self.sync_group_routes(id);
                }
            }
            Command::ActivateScene { scene_id } => {
                self.activate_scene(scene_id);
            }
            Command::ApplyProfile { stream_id, app_id } => {
                self.apply_profile_for_stream(stream_id, &app_id);
            }
            Command::SyncGroupPlugins { group_id } => self.sync_group_plugins(group_id),
            Command::StartRecording { id, group_id, path, format } => self.start_recording(id, group_id, path, format),
            Command::StopRecording { id } => self.stop_recording(id),
        }
    }

    fn create_group(&mut self, group: &Group) {
        if self.groups.contains_key(&group.id) {
            return;
        }
        let live = Arc::new(GroupLive::default());
        self.live.lock().unwrap().insert(group.id, live.clone());
        match build_group_capture(&self.core, group, self.master_gain.clone(), live, self.metrics.clone()) {
            Ok(node) => {
                tracing::info!(id = group.id, name = %group.name, "group created");
                self.groups.insert(group.id, node);
                self.sync_group_routes(group.id);
            }
            Err(err) => {
                tracing::error!(%err, id = group.id, "failed to create group");
                self.live.lock().unwrap().remove(&group.id);
            }
        }
    }

    /// Reconciles one group's live routes against its persisted
    /// `output_routes` (or the single implicit default route when that list
    /// is empty — see `RouteKey`). Called on group creation, whenever a
    /// route is added/changed/removed via D-Bus (`SyncGroupRoutes`), and
    /// whenever an output this group targets becomes available or
    /// unavailable. Never discards a route's *configuration* just because
    /// its target is momentarily unavailable
    /// (MTX_AUDIO_GROUPS_MULTI_OUTPUT.md §13) — it only tears down or
    /// (re)creates the underlying live PipeWire stream, and remembers to
    /// retry via `pending_output_routes`.
    fn sync_group_routes(&mut self, group_id: u32) {
        let wanted: Vec<GroupOutputRoute> = {
            let cfg = self.cfg.lock().unwrap();
            cfg.groups.iter().find(|g| g.id == group_id).map(|g| g.output_routes.clone()).unwrap_or_default()
        };
        let Some(group_live) = self.live.lock().unwrap().get(&group_id).cloned() else { return };
        let outputs_snapshot = self.outputs.lock().unwrap().clone();
        let core = self.core.clone();
        let metrics = self.metrics.clone();
        let max_compensation_ms = self.max_compensation_ms;

        let wanted_keys: Vec<RouteKey> =
            if wanted.is_empty() { vec![None] } else { wanted.iter().map(|r| Some(r.output_id.clone())).collect() };

        let Some(node) = self.groups.get_mut(&group_id) else { return };

        let stale: Vec<RouteKey> = node.routes.keys().filter(|k| !wanted_keys.contains(k)).cloned().collect();
        for key in &stale {
            node.routes.remove(key);
            node.route_producers.lock().unwrap().retain(|p| &p.key != key);
        }

        let mut newly_unresolved: Vec<OutputId> = Vec::new();

        if wanted.is_empty() {
            if !node.routes.contains_key(&None) {
                match create_route_node(
                    &core,
                    group_id,
                    None,
                    1.0,
                    true,
                    0.0,
                    None,
                    node.route_producers.clone(),
                    group_live.clone(),
                    metrics.clone(),
                ) {
                    Ok(route) => {
                        node.routes.insert(None, route);
                    }
                    Err(err) => tracing::error!(%err, group_id, "failed to create default route"),
                }
            }
        } else {
            for route_cfg in &wanted {
                let key: RouteKey = Some(route_cfg.output_id.clone());
                let target = outputs_snapshot.get(&route_cfg.output_id).filter(|o| o.available());
                let delay_ms = output::resolve_route_delay_ms(route_cfg, &wanted, &outputs_snapshot, max_compensation_ms);

                match node.routes.get(&key) {
                    Some(existing) => {
                        existing.gain.set_target(route_cfg.gain);
                        existing.enabled.store(route_cfg.enabled, Ordering::Relaxed);
                        existing.delay.lock().unwrap().set_delay_ms(delay_ms, SAMPLE_RATE);
                        if target.is_none() {
                            // The device just went away: drop the live
                            // stream but keep the route configuration and
                            // retry later.
                            node.routes.remove(&key);
                            node.route_producers.lock().unwrap().retain(|p| p.key != key);
                            newly_unresolved.push(route_cfg.output_id.clone());
                        }
                    }
                    None => match target {
                        Some(dev) => match create_route_node(
                            &core,
                            group_id,
                            key.clone(),
                            route_cfg.gain,
                            route_cfg.enabled,
                            delay_ms,
                            Some(dev.name.clone()),
                            node.route_producers.clone(),
                            group_live.clone(),
                            metrics.clone(),
                        ) {
                            Ok(route) => {
                                node.routes.insert(key, route);
                            }
                            Err(err) => {
                                tracing::error!(%err, group_id, output_id = %route_cfg.output_id, "failed to create output route")
                            }
                        },
                        None => newly_unresolved.push(route_cfg.output_id.clone()),
                    },
                }
            }
        }

        for output_id in newly_unresolved {
            self.pending_output_routes.entry(output_id).or_default().push(group_id);
        }
    }

    /// Retries every group waiting on `output_id` (§13/§18's reconnection
    /// flow) once it becomes available again.
    fn retry_pending_output_routes(&mut self, output_id: &OutputId) {
        if let Some(group_ids) = self.pending_output_routes.remove(output_id) {
            let mut group_ids = group_ids;
            group_ids.sort_unstable();
            group_ids.dedup();
            for group_id in group_ids {
                self.sync_group_routes(group_id);
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
    /// what any group's own (always-autoconnected) default-route playback
    /// stream is currently linked to, rather than querying PipeWire's
    /// default-sink metadata directly, since that link already has to exist
    /// for any group a stream could have been routed into in the first
    /// place.
    fn unroute_stream(&mut self, stream_id: u32) {
        self.stream_group.remove(&stream_id);
        self.sever_existing_links(stream_id);
        let hw_node = self.groups.values().find_map(|g| {
            g.routes.values().find_map(|r| {
                let playback_node = r.playback.node_id();
                self.links.values().find(|l| l.output_node == playback_node).map(|l| l.input_node)
            })
        });
        match hw_node {
            Some(hw_node) => {
                self.link_stream_to_node(stream_id, hw_node, true);
                tracing::info!(stream_id, hw_node, "unrouted stream back to hardware");
            }
            None => tracing::warn!(stream_id, "unrouted stream, but no hardware sink found to reconnect it to"),
        }
    }

    /// Reconnects every currently-routed app stream back to real hardware —
    /// called on a graceful stop (SIGTERM/SIGINT, see `run()`) so quitting
    /// the daemon behaves like removing every stream from its group, not
    /// like silently cutting them all off. Persisted `Config` routing is
    /// deliberately left untouched (unlike a manual/D-Bus unroute): on the
    /// next daemon start, `on_global`'s "honor an existing assignment" path
    /// routes each stream right back into the same group.
    fn shutdown_gracefully(&mut self) {
        let stream_ids: Vec<u32> = self.stream_group.keys().copied().collect();
        for stream_id in stream_ids {
            self.unroute_stream(stream_id);
        }
    }

    /// Registers or refreshes a discovered output device
    /// (MTX_AUDIO_GROUPS_MULTI_OUTPUT.md §11) and retries any group waiting
    /// on it.
    fn add_or_update_output(&mut self, output: OutputDevice) {
        let id = output.id.clone();
        let node_id = output.node_id;
        let mut outputs = self.outputs.lock().unwrap();
        // Matched by `node_id`, not `id` — a reclassification (see below)
        // can change `id` for the same physical device between calls, and
        // that must not look like a fresh connect for `DeviceConnected`.
        let was_available = node_id.is_some() && outputs.values().any(|o| o.node_id == node_id);
        // Re-classifying the same node under a better id (e.g. a Device's
        // `info` event fills in `device.serial` after an initial fallback-
        // by-name guess — see `discover_output_from_node`) must replace the
        // old entry, not leave it behind as a stale duplicate/ghost output
        // (confirmed live: without this, both the fallback and the correct
        // id showed up as two separate outputs for one physical device).
        if let Some(node_id) = node_id {
            outputs.retain(|existing_id, o| o.node_id != Some(node_id) || existing_id == &id);
        }
        outputs.insert(id.clone(), output);
        drop(outputs);
        self.retry_pending_output_routes(&id);
        if !was_available {
            self.fire_event(rules::Event::DeviceConnected { output_id: id });
        }
    }

    /// Marks an output unavailable rather than deleting it
    /// (MTX_AUDIO_GROUPS_MULTI_OUTPUT.md §13) — any route pointed at it gets
    /// torn down (but not forgotten) the next time its owning group is
    /// synced.
    fn mark_output_unavailable(&mut self, node_id: u32) {
        let mut outputs = self.outputs.lock().unwrap();
        let Some((id, dev)) = outputs.iter_mut().find(|(_, o)| o.node_id == Some(node_id)) else { return };
        dev.node_id = None;
        let id = id.clone();
        drop(outputs);
        let affected: Vec<u32> = {
            let cfg = self.cfg.lock().unwrap();
            cfg.groups
                .iter()
                .filter(|g| g.output_routes.iter().any(|r| r.output_id == id))
                .map(|g| g.id)
                .collect()
        };
        for group_id in affected {
            self.sync_group_routes(group_id);
        }
        self.fire_event(rules::Event::DeviceDisconnected { output_id: id });
    }

    /// Returns and clears a pending scene assignment for `app_id`, if any —
    /// see `active_scene_pending`'s doc comment.
    fn take_pending_scene_assignment(&mut self, app_id: &AppId) -> Option<u32> {
        self.active_scene_pending.remove(app_id)
    }

    /// A saved profile's preferred group for `app_id`, if any (`profiles.rs`).
    fn preferred_group_for_app(&self, app_id: &AppId) -> Option<u32> {
        self.cfg.lock().unwrap().profiles.iter().find(|p| &p.app_id == app_id).and_then(|p| p.preferred_group)
    }

    /// Applies a saved profile's volume/output preference to a stream that
    /// just appeared (`profiles.rs`) — routing to `preferred_group` is
    /// handled by the caller alongside the scene-pending/default-routing
    /// decision, since only one of those three can apply to a single new
    /// stream.
    fn apply_profile_for_stream(&mut self, stream_id: u32, app_id: &AppId) {
        let Some(profile) = self.cfg.lock().unwrap().profiles.iter().find(|p| &p.app_id == app_id).cloned() else { return };
        set_stream_volume(stream_id, profile.volume);
        let (Some(output_id), Some(group_id)) = (&profile.preferred_output, profile.preferred_group) else { return };
        let mut cfg = self.cfg.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            if !group.output_routes.iter().any(|r| &r.output_id == output_id) {
                group.output_routes.push(GroupOutputRoute::new(output_id.clone(), RouteMode::Automatic));
            }
        }
        let _ = cfg.save();
        drop(cfg);
        self.sync_group_routes(group_id);
    }

    /// Activates a scene (NewImprovenments/01-audio-scenes.md): applies
    /// every snapshotted group's DSP/routing onto the live group with the
    /// same id (skipping — and reporting — any that no longer exists), then
    /// routes whichever `stream_assignments` currently have a live match.
    /// An app not currently running is remembered in `active_scene_pending`
    /// and applied automatically the moment it starts (`on_global`) — the
    /// "partially successful rather than fail the whole scene" recovery the
    /// spec asks for.
    fn activate_scene(&mut self, scene_id: u32) -> SceneActivationReport {
        let mut report = SceneActivationReport::default();
        let Some(scene) = self.cfg.lock().unwrap().scenes.iter().find(|s| s.id == scene_id).cloned() else {
            tracing::warn!(scene_id, "activate_scene: no such scene");
            return report;
        };

        self.master_gain.set_target(scene.master_gain);
        {
            let mut cfg = self.cfg.lock().unwrap();
            cfg.master_gain = scene.master_gain;
            let _ = cfg.save();
        }

        for snapshot in &scene.groups {
            let exists = self.cfg.lock().unwrap().groups.iter().any(|g| g.id == snapshot.id);
            if !exists {
                report.missing_groups.push(snapshot.id);
                continue;
            }
            {
                let mut cfg = self.cfg.lock().unwrap();
                if let Some(g) = cfg.groups.iter_mut().find(|g| g.id == snapshot.id) {
                    g.gain = snapshot.gain;
                    g.eq_bands = snapshot.eq_bands.clone();
                    g.highpass = snapshot.highpass;
                    g.lowpass = snapshot.lowpass;
                    g.compressor = snapshot.compressor;
                    g.gate = snapshot.gate;
                    g.output_routes = snapshot.output_routes.clone();
                }
                let _ = cfg.save();
            }
            if let Some(node) = self.groups.get(&snapshot.id) {
                node.params.gain.set_target(snapshot.gain);
                for (rt, band) in node.params.eq_bands.iter().zip(snapshot.eq_bands.iter()) {
                    rt.set(band);
                }
                node.params.highpass.set(&snapshot.highpass);
                node.params.lowpass.set(&snapshot.lowpass);
                node.params.compressor.set(&snapshot.compressor);
                node.params.gate.set(&snapshot.gate);
            }
            self.sync_group_routes(snapshot.id);
            report.applied_groups.push(snapshot.id);
        }

        for (app_id, group_id) in &scene.stream_assignments {
            let live_stream = self.streams.lock().unwrap().values().find(|s| &s.app_id == app_id).map(|s| s.id);
            match live_stream {
                Some(stream_id) => {
                    self.route_and_persist(stream_id, *group_id);
                    report.applied_streams.push(app_id.clone());
                }
                None => {
                    self.active_scene_pending.insert(app_id.clone(), *group_id);
                    report.pending_streams.push(app_id.clone());
                }
            }
        }

        tracing::info!(
            scene_id,
            name = scene.name,
            applied_groups = report.applied_groups.len(),
            missing_groups = report.missing_groups.len(),
            pending_streams = report.pending_streams.len(),
            "scene activated"
        );
        self.fire_event(rules::Event::SceneActivated { scene_id });
        report
    }

    /// Evaluates every enabled rule against `event`, applying matches in
    /// ascending-priority order — 07 §"Conflict Handling"'s "highest
    /// priority wins" falls out for free this way, since the highest
    /// priority match's action is the last one applied.
    fn fire_event(&mut self, event: rules::Event) {
        if !self.rule_runtime.begin_evaluation() {
            return;
        }
        let rule_list = self.cfg.lock().unwrap().rules.clone();
        let matched: Vec<rules::Rule> = {
            let outputs = self.outputs.lock().unwrap();
            rules::matching_rules(&rule_list, &event, &self.rule_runtime, |id| outputs.get(id).is_some_and(|o| o.available()))
                .into_iter()
                .cloned()
                .collect()
        };
        for rule in matched {
            let result = self.execute_rule_actions(&rule, &event);
            let entry = rules::RuleExecution {
                rule_id: rule.id,
                rule_name: rule.name.clone(),
                trigger_summary: rules::describe_trigger(&rule.trigger),
                actions_summary: rules::describe_actions(&rule.actions),
                ok: result.is_ok(),
                message: result.err().unwrap_or_default(),
            };
            tracing::info!(rule_id = rule.id, name = rule.name, ok = entry.ok, "rule fired");
            self.rule_runtime.mark_fired(rule.id, entry);
        }
        self.rule_runtime.end_evaluation();
    }

    fn execute_rule_actions(&mut self, rule: &rules::Rule, event: &rules::Event) -> Result<(), String> {
        for action in &rule.actions {
            match action {
                rules::Action::RouteTriggeringAppToGroup { group_id } => {
                    let rules::Event::AppStarted { stream_id, .. } = event else {
                        return Err("requires an app-start trigger".to_owned());
                    };
                    self.route_and_persist(*stream_id, *group_id);
                }
                rules::Action::RouteGroupToOutput { group_id, output_id, gain } => {
                    let mut cfg = self.cfg.lock().unwrap();
                    let Some(group) = cfg.groups.iter_mut().find(|g| g.id == *group_id) else {
                        return Err(format!("no such group {group_id}"));
                    };
                    match group.output_routes.iter_mut().find(|r| &r.output_id == output_id) {
                        Some(r) => {
                            r.gain = *gain;
                            r.enabled = true;
                        }
                        None => {
                            let mut route = GroupOutputRoute::new(output_id.clone(), RouteMode::Automatic);
                            route.gain = *gain;
                            group.output_routes.push(route);
                        }
                    }
                    let _ = cfg.save();
                    drop(cfg);
                    self.sync_group_routes(*group_id);
                }
                rules::Action::SetGroupVolume { group_id, gain } => {
                    let mut cfg = self.cfg.lock().unwrap();
                    if let Some(g) = cfg.groups.iter_mut().find(|g| g.id == *group_id) {
                        g.gain = *gain;
                    }
                    let _ = cfg.save();
                    drop(cfg);
                    if let Some(node) = self.groups.get(group_id) {
                        node.params.gain.set_target(*gain);
                    }
                }
                rules::Action::ActivateScene { scene_id } => {
                    self.activate_scene(*scene_id);
                }
                rules::Action::ApplyProfileForTriggeringApp => {
                    let (app_id, stream_id) = match event {
                        rules::Event::AppStarted { app_id, stream_id } => (app_id.clone(), *stream_id),
                        _ => return Err("requires an app-start trigger".to_owned()),
                    };
                    self.apply_profile_for_stream(stream_id, &app_id);
                }
            }
        }
        Ok(())
    }

    /// Called from a ~20ms PipeWire timer (`run()`) to evaluate every
    /// ducking rule and write each affected group's new duck-gain target
    /// (`ducking.rs`'s module doc explains why this can't live inside a
    /// single group's own audio callback).
    fn evaluate_ducking(&mut self, tick_seconds: f32) {
        let rule_list = self.cfg.lock().unwrap().ducking_rules.clone();
        if rule_list.is_empty() {
            return;
        }
        let targets = {
            let live = self.live.lock().unwrap();
            self.duck_engine.evaluate(
                &rule_list,
                |group_id| {
                    live.get(&group_id).map(|l| {
                        let (level_l, level_r, _, _) = l.read();
                        level_l.max(level_r)
                    })
                },
                tick_seconds,
            )
        };
        for (group_id, gain) in targets {
            if let Some(node) = self.groups.get(&group_id) {
                node.params.duck_gain.set_target(gain);
            }
        }
    }

    /// Rebuilds `group_id`'s live plugin chain from its persisted
    /// `Group.plugins` (`plugins.rs`) — called after any D-Bus plugin
    /// add/remove/reorder/param-change.
    fn sync_group_plugins(&mut self, group_id: u32) {
        let instances = {
            let cfg = self.cfg.lock().unwrap();
            let Some(group) = cfg.groups.iter().find(|g| g.id == group_id) else { return };
            group.plugins.clone()
        };
        let Some(node) = self.groups.get(&group_id) else { return };
        *node.plugin_chain.lock().unwrap() = plugins::PluginChain::build(&instances, SAMPLE_RATE);
    }

    /// Starts recording `group_id`'s post-DSP-and-plugins signal to `path`
    /// (`recording.rs`) — registers the ring buffer's producer half into
    /// that group's `recording_producers` fan-out so the capture callback
    /// starts feeding it from the very next buffer.
    fn start_recording(&mut self, id: u32, group_id: u32, path: std::path::PathBuf, format: crate::recording::RecordingFormat) {
        let Some(node) = self.groups.get(&group_id) else {
            tracing::warn!(group_id, "start_recording for unknown group");
            return;
        };
        match crate::recording::start(id, group_id, path.clone(), format) {
            Ok((session, producer)) => {
                node.recording_producers.lock().unwrap().push(RecordingProducer { id, producer });
                self.recording_status.lock().unwrap().insert(
                    id,
                    RecordingStatus { group_id, path: path.display().to_string(), started_at: session.started_at(), frames_written_bits: session.frames_written_bits() },
                );
                self.recordings.insert(id, session);
                tracing::info!(id, group_id, path = %path.display(), "recording started");
            }
            Err(err) => tracing::error!(%err, group_id, path = %path.display(), "failed to start recording"),
        }
    }

    fn stop_recording(&mut self, id: u32) {
        if let Some(node) = self.recordings.get(&id).and_then(|s| self.groups.get(&s.group_id)) {
            node.recording_producers.lock().unwrap().retain(|r| r.id != id);
        }
        self.recording_status.lock().unwrap().remove(&id);
        if let Some(session) = self.recordings.remove(&id) {
            session.stop_and_finalize();
            tracing::info!(id, "recording stopped");
        }
    }

    /// Stops and finalizes every active recording — called from the
    /// graceful-shutdown path (`run()`'s SIGTERM/SIGINT handlers) so a
    /// daemon stop doesn't leave a headerless WAV file behind (same
    /// concern, same fix shape as `shutdown_gracefully`'s stream
    /// reconnection).
    fn finalize_all_recordings(&mut self) {
        let ids: Vec<u32> = self.recordings.keys().copied().collect();
        for id in ids {
            self.stop_recording(id);
        }
    }

    /// Stops and finalizes any recording tied to `group_id` — must run
    /// *before* that group's `GroupNode` (and the `recording_producers`
    /// list it owns) is removed. Without this, deleting a group mid-
    /// recording drops the ring buffer's producer half but never signals
    /// the writer thread to stop: it would spin on `PopError::Empty`
    /// forever, leaking a thread and leaving the `.tmp` file un-finalized
    /// for the rest of the daemon's life (found via code review, not a
    /// live crash — same category of bug as the graceful-shutdown fix
    /// above, just triggered by group deletion instead of process exit).
    fn finalize_recordings_for_group(&mut self, group_id: u32) {
        let ids: Vec<u32> = self.recordings.iter().filter(|(_, s)| s.group_id == group_id).map(|(&id, _)| id).collect();
        for id in ids {
            self.stop_recording(id);
        }
    }
}

/// Sets a raw PipeWire stream's own volume via `wpctl` — same "shell out to
/// the already-trusted tool instead of hand-building an SPA Props pod"
/// choice `dbus.rs::set_stream_muted` already makes, reused here since a
/// profile's volume is that exact same per-stream property. Called from the
/// PipeWire engine thread (a new app stream appearing, an infrequent
/// control event, not the audio callback itself) — same tolerated-blocking-
/// control-op category this file's own module doc already accepts for e.g.
/// `Command` handling sharing the mainloop thread with audio callbacks.
fn set_stream_volume(stream_id: u32, linear: f32) {
    let _ = std::process::Command::new("wpctl").args(["set-volume", &stream_id.to_string(), &format!("{:.4}", linear.max(0.0))]).status();
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

/// Builds just a group's capture side (the virtual `Audio/Sink` app streams
/// route into) plus the shared DSP params and route-producer fan-out list.
/// Playback/output routes are built separately by `create_route_node` and
/// reconciled by `Engine::sync_group_routes` — a group always has at least
/// one route once `sync_group_routes` runs once, but has zero at this point.
fn build_group_capture(
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

    let route_producers: Arc<Mutex<Vec<RouteProducer>>> = Arc::new(Mutex::new(Vec::new()));
    let plugin_chain = Arc::new(Mutex::new(plugins::PluginChain::build(&group.plugins, SAMPLE_RATE)));
    let recording_producers: Arc<Mutex<Vec<RecordingProducer>>> = Arc::new(Mutex::new(Vec::new()));

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
    let capture_metrics_live = live.clone();
    let capture_metrics = metrics.clone();
    let capture_route_producers = route_producers.clone();
    let capture_plugin_chain = plugin_chain.clone();
    let capture_recording_producers = recording_producers.clone();
    let capture_listener = capture
        .add_local_listener_with_user_data(())
        .process(move |stream, _| {
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }
            capture_metrics_live.mark_active();
            let data = &mut datas[0];
            let stride = CHANNELS * std::mem::size_of::<f32>();
            let n_frames = data.chunk().size() as usize / stride;
            // Regression fix: this was dropped when the original single
            // capture+playback stream got split into `build_group_capture`
            // + `create_route_node` (Phase 1's multi-output work) — the
            // playback side kept its own `metrics` for underrun counting,
            // but the capture side's buffer-size report was lost in the
            // split. Confirmed live: the Home tab's "Buffer" tile and MTX
            // Doctor's "Audio buffer" check both silently read a
            // never-updated value once this went missing.
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
            let mut producers = capture_route_producers.lock().unwrap();
            let mut recordings = capture_recording_producers.lock().unwrap();
            let mut plugin_chain = capture_plugin_chain.lock().unwrap();
            for frame in 0..n_frames {
                // ponytail: every group's closure ticks the shared master
                // ramp once per frame, so with N groups active it converges
                // ~N times faster than PARAM_RAMP_MS states — harmless (still
                // smooth, just brisker) rather than a race, since everything
                // here runs on the one pw mainloop thread. Fix path: apply
                // master gain once at a single shared mix point instead of
                // once per group, if that speed-up becomes audible.
                let gain = capture_params.gain.next() * capture_params.master_gain.next() * capture_params.duck_gain.next();
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

                // Plugin chain slots in here (NewImprovenments/
                // 10-plugin-dsp-architecture.md's `Gain -> EQ -> Filter ->
                // Compressor -> Gate -> Plugin(s) -> Output`) — after every
                // built-in stage, before the signal fans out to routes and
                // recordings, so both hear exactly what a plugin did to it.
                let mut frame = [samples[0] * mult, samples[1] * mult];
                plugin_chain.process(&mut frame);

                for (ch, out) in frame.into_iter().enumerate() {
                    buf_peak[ch] = buf_peak[ch].max(out.abs());
                    for route in producers.iter_mut() {
                        let _ = route.producer.push(out);
                    }
                    for rec in recordings.iter_mut() {
                        let _ = rec.producer.push(out);
                    }
                }
            }
            drop(producers);
            drop(recordings);
            live.publish(buf_peak[0], buf_peak[1], dynamics.gain_reduction_db(), dynamics.gate_reduction_db());
            if let Some(bins) = spectrum.maybe_compute() {
                live.publish_spectrum(&bins);
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

    Ok(GroupNode {
        capture,
        params,
        route_producers,
        routes: HashMap::new(),
        plugin_chain,
        recording_producers,
        _capture_listener: capture_listener,
    })
}

/// Builds one route's playback stream: its own gain/delay/enable state
/// (MTX_AUDIO_GROUPS_MULTI_OUTPUT.md §7/§8) feeding real output hardware,
/// bridged from the group's shared post-DSP signal by a fresh ring buffer
/// whose producer half is registered into `route_producers` for the
/// capture closure to fan out into.
///
/// `target_node_name`: `None` connects like the original single-playback-
/// stream design did — bare `AUTOCONNECT`, whatever PipeWire's session
/// manager currently calls the default sink (used for the implicit default
/// route, `RouteKey::None`, so a group with no explicit output routing keeps
/// behaving exactly as before this feature existed). `Some(name)` targets
/// that specific output by its PipeWire `node.name` via `TARGET_OBJECT`,
/// still with `AUTOCONNECT` set so the session manager resolves it (rather
/// than us hand-linking ports, which the capture side does for a different
/// reason — its virtual sink isn't something the session manager targets on
/// its own).
#[allow(clippy::too_many_arguments)]
fn create_route_node(
    core: &pw::core::CoreRc,
    group_id: u32,
    key: RouteKey,
    initial_gain: f32,
    initial_enabled: bool,
    initial_delay_ms: f32,
    target_node_name: Option<String>,
    route_producers: Arc<Mutex<Vec<RouteProducer>>>,
    live: Arc<GroupLive>,
    metrics: Arc<Metrics>,
) -> Result<RouteNode, pw::Error> {
    let route_label = key.clone().unwrap_or_else(|| "default".to_owned());
    let stream_name = format!("mtxpanel.group.{group_id}.route.{route_label}");

    let mut props = properties! {
        *keys::MEDIA_TYPE => "Audio",
        *keys::MEDIA_CATEGORY => "Playback",
        *keys::MEDIA_ROLE => "Music",
        *keys::NODE_NAME => stream_name.as_str(),
        *keys::NODE_PASSIVE => "true",
    };
    if let Some(target) = &target_node_name {
        props.insert(*keys::TARGET_OBJECT, target.as_str());
    }
    let playback = pw::stream::StreamRc::new(core.clone(), &stream_name, props)?;

    let gain = Arc::new(Smoothed::new(initial_gain, SAMPLE_RATE, PARAM_RAMP_MS));
    let enabled = Arc::new(AtomicBool::new(initial_enabled));
    let max_delay_samples = (ROUTE_DELAY_BUFFER_MS * 0.001 * SAMPLE_RATE) as usize;
    let delay = Arc::new(Mutex::new(RouteDelay::new(max_delay_samples)));
    delay.lock().unwrap().set_delay_ms(initial_delay_ms, SAMPLE_RATE);

    // A route re-created after its device disappeared and came back must
    // not leave a stale producer (for the consumer that got dropped along
    // with the old stream) sitting in the shared fan-out list.
    route_producers.lock().unwrap().retain(|p| p.key != key);
    let (ring_tx, mut ring_rx) = rtrb::RingBuffer::<f32>::new(RING_CAPACITY);
    route_producers.lock().unwrap().push(RouteProducer { key: key.clone(), producer: ring_tx });

    let route_gain = gain.clone();
    let route_enabled = enabled.clone();
    let route_delay = delay.clone();
    let route_metrics = metrics.clone();
    let route_live = live.clone();
    let listener = playback
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

            let is_enabled = route_enabled.load(Ordering::Relaxed);
            let mut delay_lines = route_delay.lock().unwrap();
            for frame in 0..n_frames {
                let g = route_gain.next();
                for ch in 0..CHANNELS {
                    let sample = ring_rx.pop().unwrap_or_else(|_| {
                        // Only counts once this group has ever actually
                        // received app audio — see `GroupLive::has_input`'s
                        // doc comment for why.
                        if route_live.has_input() {
                            route_metrics.underruns.fetch_add(1, Ordering::Relaxed);
                        }
                        0.0
                    });
                    let delayed = delay_lines.lines[ch].process(sample);
                    let out = if is_enabled { delayed * g } else { 0.0 };
                    let start = frame * stride + ch * 4;
                    slice[start..start + 4].copy_from_slice(&out.to_le_bytes());
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
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS,
        &mut params_vec,
    )?;

    Ok(RouteNode { playback, gain, enabled, delay, _listener: listener })
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    cfg: Config,
    cmd_rx: pw::channel::Receiver<Command>,
    streams: Arc<Mutex<HashMap<u32, AppStream>>>,
    live: Arc<Mutex<HashMap<u32, Arc<GroupLive>>>>,
    metrics: Arc<Metrics>,
    cfg_state: Arc<Mutex<Config>>,
    outputs: Arc<Mutex<HashMap<OutputId, OutputDevice>>>,
    recording_status: Arc<Mutex<HashMap<u32, RecordingStatus>>>,
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
        outputs,
        devices: HashMap::new(),
        device_proxies: HashMap::new(),
        sink_nodes: HashMap::new(),
        pending_output_routes: HashMap::new(),
        system_default_sink: None,
        // Re-clamped on load too, not just in dbus.rs's setter — defends
        // against a config file written before that clamp existed, or
        // hand-edited past the real buffer limit.
        max_compensation_ms: cfg.max_compensation_ms.clamp(0.0, ROUTE_DELAY_BUFFER_MS),
        streams: streams.clone(),
        rule_runtime: rules::RuleRuntime::default(),
        active_scene_pending: HashMap::new(),
        duck_engine: ducking::DuckingEngine::default(),
        recordings: HashMap::new(),
        recording_status,
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
            let removed_app_id = streams.lock().unwrap().remove(&id).map(|s| s.app_id);
            let mut engine = engine_for_remove.borrow_mut();
            if let Some(app_id) = removed_app_id {
                engine.fire_event(rules::Event::AppStopped { app_id });
            }
            engine.ports.remove(&id);
            engine.links.remove(&id);
            engine.stream_nodes.remove(&id);
            engine.devices.remove(&id);
            engine.device_proxies.remove(&id);
            engine.sink_nodes.remove(&id);
            // A stream that closed before its route ever completed (tab
            // shut fast) would otherwise sit in here forever, retried
            // pointlessly on every later port event.
            engine.pending_routes.remove(&id);
            engine.stream_group.remove(&id);
            engine.mark_output_unavailable(id);
            // Otherwise a closed app's stream id lingers in its old group's
            // routed_streams forever — persisted, so it outlives restarts —
            // and the UI has nothing live to look its name up against,
            // showing a dead "stream #N" instead of just dropping the row.
            let mut cfg = engine.cfg.lock().unwrap();
            for group in cfg.groups.iter_mut() {
                group.routed_streams.retain(|&sid| sid != id);
            }
            let _ = cfg.save();
        })
        .register();

    let engine_for_cmd = engine.clone();
    let _cmd_receiver = cmd_rx.attach(mainloop.loop_(), move |cmd| {
        engine_for_cmd.borrow_mut().handle_command(cmd);
    });

    // A graceful stop (systemd stop/restart, Ctrl-C) must not strand live
    // audio the way a hard kill does: without our virtual nodes, nothing
    // else automatically relinks the app streams they fed back to hardware
    // (observed live — a killed daemon left a playing app's stream sitting
    // unlinked, silent, until manually relinked). `shutdown_gracefully`
    // reuses the exact same reconnect-to-hardware path `unroute_stream`
    // already uses for a manual "remove from group" — but that path only
    // *requests* link creation (`Core::create_object` is async); quitting
    // immediately after, as a first version of this did, let the process
    // exit before the request ever reached the server, silently dropping it
    // despite `object.linger` (also observed live: the log claimed success,
    // `pw-link` showed no link). `core.sync()` + a `done` listener waits for
    // the server to actually confirm it caught up before we quit.
    let engine_for_sigterm = engine.clone();
    let core_for_sigterm = core.clone();
    let mainloop_for_sigterm = mainloop.clone();
    let _sigterm = mainloop.loop_().add_signal_local(pw::loop_::Signal::TERM, move || {
        tracing::info!("received SIGTERM, reconnecting routed streams to hardware before exit");
        engine_for_sigterm.borrow_mut().shutdown_gracefully();
        engine_for_sigterm.borrow_mut().finalize_all_recordings();
        let mainloop_for_done = mainloop_for_sigterm.clone();
        let listener = core_for_sigterm.add_listener_local().done(move |_id, _seq| mainloop_for_done.quit()).register();
        std::mem::forget(listener); // process exits moments after this fires; nothing left to clean up
        let _ = core_for_sigterm.sync(0);
    });
    let engine_for_sigint = engine.clone();
    let core_for_sigint = core.clone();
    let mainloop_for_sigint = mainloop.clone();
    let _sigint = mainloop.loop_().add_signal_local(pw::loop_::Signal::INT, move || {
        tracing::info!("received SIGINT, reconnecting routed streams to hardware before exit");
        engine_for_sigint.borrow_mut().shutdown_gracefully();
        engine_for_sigint.borrow_mut().finalize_all_recordings();
        let mainloop_for_done = mainloop_for_sigint.clone();
        let listener = core_for_sigint.add_listener_local().done(move |_id, _seq| mainloop_for_done.quit()).register();
        std::mem::forget(listener);
        let _ = core_for_sigint.sync(0);
    });

    // Ducking (`ducking.rs`) is evaluated on a fixed ~20ms tick rather than
    // per audio buffer — it needs to compare *other* groups' live levels
    // against each other, state a single group's own self-contained
    // process() closure doesn't have access to.
    let engine_for_duck = engine.clone();
    let duck_timer = mainloop.loop_().add_timer(move |_expirations| {
        engine_for_duck.borrow_mut().evaluate_ducking(0.02);
    });
    duck_timer.update_timer(Some(Duration::from_millis(20)), Some(Duration::from_millis(20)));

    tracing::info!("connected to pipewire, watching for app streams (ctrl-c to quit)");
    mainloop.run();

    Ok(())
}

/// PipeWire's "default" metadata object reports e.g. `default.audio.sink` as
/// a JSON string like `{"name":"alsa_output.xyz"}` — this pulls just the
/// name out without pulling in a JSON crate for one field.
fn extract_json_name(value: &str) -> Option<String> {
    let idx = value.find("\"name\"")?;
    let after = &value[idx + 6..];
    let colon = after.find(':')?;
    let after_colon = after[colon + 1..].trim_start();
    let rest = after_colon.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
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

    if global.type_ == pw::types::ObjectType::Device {
        // Confirmed live (`device.bus`/`form-factor`/vendor-product/serial
        // all came back missing until this was added): a Device *global*'s
        // own registry-announced properties are minimal — that richer data
        // only arrives via the Device's own `info` event after binding it,
        // the same two-step "global add, then bind for the real info" shape
        // already used below for a Node's `running` state.
        let device_id = global.id;
        let Ok(device) = engine.borrow().registry.bind::<pw::device::Device, _>(global) else {
            tracing::warn!(device_id, "failed to bind device for property info");
            return;
        };
        let engine_for_info = engine.clone();
        let listener = device
            .add_listener_local()
            .info(move |info| {
                let Some(info_props) = info.props() else { return };
                let mut engine = engine_for_info.borrow_mut();
                engine.devices.insert(device_id, snapshot_props(info_props));
                // A Sink Node's registry event can arrive before its parent
                // Device's `info` event (this one is doubly async — bind,
                // then wait for the server's info reply) — any already-
                // cached Sink node referencing this device gets
                // re-classified now that the data it needed exists.
                let waiting: Vec<u32> = engine
                    .sink_nodes
                    .iter()
                    .filter(|(_, props)| props.get("device.id").and_then(|s| s.parse::<u32>().ok()) == Some(device_id))
                    .map(|(&node_id, _)| node_id)
                    .collect();
                for node_id in waiting {
                    discover_output_from_node(&mut engine, node_id);
                }
            })
            .register();
        engine.borrow_mut().device_proxies.insert(device_id, (device, listener));
        return;
    }

    if global.type_ == pw::types::ObjectType::Metadata {
        if props.get("metadata.name") != Some("default") {
            return;
        }
        let Ok(metadata) = engine.borrow().registry.bind::<pw::metadata::Metadata, _>(global) else { return };
        let engine_for_default = engine.clone();
        let listener = metadata
            .add_listener_local()
            .property(move |_subject, key, _type, value| {
                if key == Some("default.audio.sink") {
                    let name = value.and_then(extract_json_name);
                    let mut engine = engine_for_default.borrow_mut();
                    engine.system_default_sink = name.and_then(|name| {
                        engine.outputs.lock().unwrap().values().find(|o| o.name == name).map(|o| o.id.clone())
                    });
                }
                0
            })
            .register();
        // Leaked deliberately: this object and listener live for the
        // daemon's whole lifetime (there is exactly one "default" metadata
        // object per PipeWire session), and `Engine` doesn't otherwise track
        // per-global proxies for anything except app streams
        // (`stream_nodes`) — adding a single-purpose field just to hold one
        // never-removed entry isn't worth it.
        std::mem::forget((metadata, listener));
        return;
    }

    if global.type_ == pw::types::ObjectType::Node {
        if let Some(media_class) = props.get("media.class") {
            if media_class == "Audio/Sink" || media_class == "Audio/Duplex" {
                let node_name = props.get("node.name").unwrap_or("unknown-output");
                if node_name.starts_with("mtxpanel.") {
                    return; // our own route playback nodes, not real outputs
                }
                let mut engine = engine.borrow_mut();
                engine.sink_nodes.insert(global.id, snapshot_props(props));
                discover_output_from_node(&mut engine, global.id);
                return;
            }
        }
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
    let app_id = identity::app_identity(props);
    tracing::info!(id = global.id, name, service, %app_id, "app stream");
    streams.lock().unwrap().insert(global.id, AppStream { id: global.id, name, service, running: false, app_id: app_id.clone() });
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
        None => {
            // Genuinely new — not a restart-replay of an already-known
            // assignment. This is exactly the moment automation should act:
            // a pending scene assignment (a scene activated while this app
            // wasn't running yet) takes priority, then a saved profile's
            // preferred group, then the original catch-all default — and
            // only a genuinely new arrival fires rule/profile automation,
            // so a daemon restart replaying already-known streams never
            // re-fires a rule meant for a fresh app launch.
            let mut engine_mut = engine.borrow_mut();
            match engine_mut.take_pending_scene_assignment(&app_id) {
                Some(group_id) => engine_mut.route_and_persist(global.id, group_id),
                None => match engine_mut.preferred_group_for_app(&app_id) {
                    Some(group_id) => engine_mut.route_and_persist(global.id, group_id),
                    None => engine_mut.auto_route_default(global.id),
                },
            }
            engine_mut.apply_profile_for_stream(global.id, &app_id);
            drop(engine_mut);
            engine.borrow_mut().fire_event(rules::Event::AppStarted { app_id: app_id.clone(), stream_id: global.id });
        }
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

    #[test]
    fn extract_json_name_pulls_the_name_field() {
        assert_eq!(extract_json_name(r#"{"name":"alsa_output.pci-0000_00_1f.3"}"#), Some("alsa_output.pci-0000_00_1f.3".to_owned()));
        assert_eq!(extract_json_name(r#"{"name": "bluez_output.AA_BB"}"#), Some("bluez_output.AA_BB".to_owned()));
        assert_eq!(extract_json_name("null"), None);
    }
}
