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
use crate::dsp::dynamics::{CompressorParams, DynamicsProcessor, GateParams, Limiter};
use crate::dsp::smoothed::Smoothed;
use crate::dsp::spectrum::SpectrumAnalyzer;
use crate::group::{Group, GroupOutputRoute};
use crate::identity::{self, AppId, OutputId};
use crate::output::{self, OutputDevice, RouteMode};
use crate::plugins;
use crate::rules;

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

/// Every currently-live stream sharing `app_id` — deliberately a `Vec`, not
/// an `Option`. An `AppId` is many-to-one with live PipeWire streams: a
/// multi-tab browser (Firefox included) doesn't set a distinguishing
/// `application.id` per tab, so `identity::app_identity`'s own documented
/// fallback collapses every tab of one browser window to the exact same
/// identity. Any feature that resolves "the stream(s) for this app" —
/// Scene activation, applying a saved Profile — must act on *every* match,
/// not just the first: using a single-match lookup here was a real,
/// reproduced bug (confirmed live) that silently routed only one of
/// several open tabs and left the others stranded, connected directly to
/// hardware instead of through the mixer, every time it ran.
pub fn streams_matching_app_id(streams: &HashMap<u32, AppStream>, app_id: &AppId) -> Vec<u32> {
    streams.values().filter(|s| &s.app_id == app_id).map(|s| s.id).collect()
}

/// Pulls a short "service/site name" out of a stream's `media.name` (a
/// browser tab title, most often) so the Inputs tab can show "YouTube"
/// instead of a full, ever-changing video title. Two patterns cover what's
/// actually observed live: a leading unread-count badge ("(1) ...", "(57)
/// WhatsApp") gets stripped, then the trailing "Title <sep> Site" segment
/// (browser tabs conventionally suffix the site name) is taken as the
/// service name — checking both " - " (YouTube) and " | " (Udemy; live-
/// verified against a real tab title with no dash at all, which used to
/// fall through and return the *entire* title as the "service") and taking
/// whichever separator's last occurrence is furthest right, so a title
/// containing both still ends on the real site name; with neither
/// separator present, the whole (count-stripped) title already *is* the
/// service name (single-page apps like WhatsApp Web/Spotify Web just set
/// their own name as the title).
fn extract_service_name(media_name: &str) -> Option<String> {
    let stripped = media_name
        .strip_prefix('(')
        .and_then(|rest| rest.split_once(')'))
        .map(|(_, rest)| rest.trim_start())
        .unwrap_or(media_name);
    if stripped.is_empty() {
        return None;
    }
    let last_sep = [" - ", " | "]
        .iter()
        .filter_map(|sep| stripped.rfind(sep).map(|pos| (pos, sep.len())))
        .max_by_key(|&(pos, _)| pos);
    let service = match last_sep {
        Some((pos, sep_len)) => &stripped[pos + sep_len..],
        None => stripped,
    }
    .trim();
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
/// dequeue when something is both linked in *and* actually producing.
/// Without this gate, an idle group's ring buffer starves from the moment
/// it exists, and `underruns` (Metrics) counts millions of "misses" that
/// are really just silence — not the audible glitches the metric is meant
/// to flag.
///
/// `capture_ticks` is a plain counter the capture closure bumps every cycle
/// it actually dequeues a real (non-empty) buffer — i.e. "is my own
/// pipeline currently relaying anything at all right now", read lock-free
/// from the real-time playback closure, which treats capture as "alive" as
/// long as this counter has moved within the last few of *its own* cycles
/// (a few quanta of slack for normal capture/playback scheduling jitter —
/// the two streams aren't linked to each other in the graph, so PipeWire
/// gives no ordering guarantee between them).
///
/// This replaced two earlier, both-wrong designs, in order:
/// 1. A one-way `has_input` latch that only ever flipped true and never
///    back off — confirmed live: a group whose streams were all moved
///    elsewhere kept "underrunning" at hundreds to thousands of misses/sec
///    forever afterward, since its now-empty route never stopped being
///    asked to produce audio it would never receive again.
/// 2. `has_active_streams`, a control-plane flag ("is a stream routed
///    here, and is it `NodeState::Running`") kept current by `Engine` on
///    routing/running-state changes. Also confirmed live as wrong, twice
///    over: (a) a stream can sit at `Running` for a long stretch while
///    genuinely producing nothing — PipeWire simply stops scheduling
///    capture cycles for it meanwhile — so the gate stayed "active" while
///    the ring buffer was, correctly, completely empty; live-verified with
///    a real routed browser tab logging repeated 100%-miss callbacks
///    (1024/1024 samples) while marked active. (b) a stream that reaches
///    EOF and gets torn down can leave a real gap between "capture stops
///    receiving data" and "the removal event updates the flag" — live-
///    verified as a ~76,800-sample burst in under a second, right as a
///    test stream ended. Both are exactly the same underlying mistake:
///    inferring "should data be flowing" from *any* control-plane state
///    instead of asking the capture side directly whether it is.
pub struct GroupLive {
    level_l_bits: AtomicU32,
    level_r_bits: AtomicU32,
    /// Pre-fader input peak — what ducking listens to, so a trigger
    /// group's own fader never decides whether others duck.
    input_peak_bits: AtomicU32,
    gr_db_bits: AtomicU32,
    gate_gr_db_bits: AtomicU32,
    capture_ticks: AtomicU64,
    /// One `AtomicU32` (f32 bits) per spectrum bin — same lock-free idiom
    /// as everything else here, just an array of it instead of one value.
    spectrum: [AtomicU32; crate::dsp::spectrum::N_BINS],
}

impl Default for GroupLive {
    fn default() -> Self {
        Self {
            level_l_bits: AtomicU32::new(0),
            level_r_bits: AtomicU32::new(0),
            input_peak_bits: AtomicU32::new(0),
            gr_db_bits: AtomicU32::new(0),
            gate_gr_db_bits: AtomicU32::new(0),
            capture_ticks: AtomicU64::new(0),
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

    fn publish_input_peak(&self, peak: f32) {
        self.input_peak_bits.store(peak.to_bits(), Ordering::Relaxed);
    }

    pub fn input_peak(&self) -> f32 {
        f32::from_bits(self.input_peak_bits.load(Ordering::Relaxed))
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

    /// Bumped by the capture closure itself every cycle it dequeues a real
    /// buffer — see this struct's own doc comment.
    fn record_capture_tick(&self) {
        self.capture_ticks.fetch_add(1, Ordering::Relaxed);
    }

    fn capture_ticks(&self) -> u64 {
        self.capture_ticks.load(Ordering::Relaxed)
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
    /// Master bus counterpart to `SyncGroupRoutes` — sent after `Config`'s
    /// `master_output_routes` changes (add/edit/remove a master route).
    SyncMasterRoutes,
    SetMaxCompensationMs(f32),
    /// Settings tab "audio device selection" (Section 7.5): re-targets
    /// every group's *implicit* default route (the one used when it has no
    /// explicit `output_routes` of its own) at the newly-preferred device —
    /// unlike `SyncGroupRoutes`, this force-recreates that route even
    /// though its key hasn't changed, since the whole point is for an
    /// already-playing group to audibly move, not wait for some unrelated
    /// event to happen to recreate it.
    SetPreferredDefaultOutput,
    ActivateScene { scene_id: u32 },
    /// Rebuilds a group's live plugin chain from its persisted
    /// `Group.plugins` list (`plugins.rs`) — same "tell the engine to
    /// catch up after `Config` already changed" shape as `SyncGroupRoutes`.
    SyncGroupPlugins { group_id: u32 },
    StartRecording { id: u32, group_id: u32, path: std::path::PathBuf, format: crate::recording::RecordingFormat },
    StopRecording { id: u32 },
    /// logind reported wake from suspend (`main.rs::watch_resume`).
    SystemResume,
    /// `Group::feeds_group` changed: rebuild the group's output route,
    /// since its target node changed.
    RebuildGroupRoutes { group_id: u32 },
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
    /// `target_node_name` it was created with — a default route is rebuilt
    /// when the wanted target changes (see `sync_group_routes`).
    target: Option<String>,
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
    /// One tap per currently-live Master route (`Engine::sync_master_routes`)
    /// — same signal, same per-sample fan-out loop as `route_producers`/
    /// `recording_producers` above, reusing `RouteProducer`'s shape with
    /// `key` holding the master route's `OutputId` (never `None`, unlike a
    /// group's own routes) purely to identify which master route this tap
    /// feeds when tearing one down.
    master_taps: Arc<Mutex<Vec<RouteProducer>>>,
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
/// once when the Node itself is first seen, and again every time that
/// Device's `info` event fires (see the Device branch above) — which, live-
/// confirmed, is not a one-shot "here's your data" callback but fires
/// repeatedly over a device's lifetime, sometimes with a partial property
/// set that lacks `device.serial`/vendor+product id even after a prior call
/// already had them (observed live: routing to a group's own explicit
/// output *and* a freshly added Master route both went silently unresolved
/// because the id a route was saved under had since flapped to a different
/// one). Once a node has resolved to a real hardware id (anything but the
/// `name:`-prefixed last-resort fallback — see `identity::output_identity`),
/// a later call that would only manage a worse guess keeps the existing one
/// instead — every *other* field (kind, latency, sample rate, etc.) still
/// updates normally, just never the id backwards.
fn discover_output_from_node(engine: &mut Engine, node_id: u32) {
    let Some(node_props) = engine.sink_nodes.get(&node_id).cloned() else { return };
    let device_props = node_props.get("device.id").and_then(|s| s.parse::<u32>().ok()).and_then(|id| engine.devices.get(&id));

    let node_name = node_props.get("node.name").map(String::as_str).unwrap_or("unknown-output");
    let computed_id = identity::output_identity(&node_props, device_props);
    let existing_better_id = engine
        .outputs
        .lock()
        .unwrap()
        .values()
        .find(|o| o.node_id == Some(node_id) && !o.id.starts_with("name:"))
        .map(|o| o.id.clone());
    let id = match existing_better_id {
        Some(better) if computed_id.starts_with("name:") => better,
        _ => computed_id,
    };
    let kind = identity::output_kind(&node_props, device_props);
    let is_system_default = engine.system_default_sink.as_deref() == Some(id.as_str());
    let latency = identity::estimate_output_latency(&node_props);
    let description = node_props.get("node.description").map(String::as_str).unwrap_or(node_name).to_owned();
    let sample_rate = node_props.get("audio.rate").and_then(|s| s.parse().ok()).unwrap_or(SAMPLE_RATE as u32);
    let channels = node_props.get("audio.channels").and_then(|s| s.parse().ok()).unwrap_or(CHANNELS as u32);

    // A Device's `info` event can fire far more often than its data ever
    // actually changes — live-confirmed as a real storm, not a rare edge
    // case: dozens of these events within a couple of seconds, all
    // resolving to the exact same classification. Re-running
    // `add_or_update_output` (which replaces the whole map entry and fires
    // a fresh `DeviceConnected`-adjacent log line) on every single one
    // wastes real CPU on the PipeWire mainloop thread for zero behavior
    // change — confirmed live as more than a cosmetic cost: a storm like
    // this coincided with the D-Bus service becoming briefly unresponsive.
    // Skip the update (and the log line) entirely when nothing this
    // function computed actually differs from what's already stored.
    {
        let outputs = engine.outputs.lock().unwrap();
        if let Some(existing) = outputs.get(&id) {
            let unchanged = existing.kind == kind
                && existing.node_id == Some(node_id)
                && existing.is_system_default == is_system_default
                && existing.sample_rate == sample_rate
                && existing.channels == channels
                && existing.latency == latency
                && existing.name == node_name
                && existing.description == description;
            if unchanged {
                return;
            }
        }
    }

    let output = OutputDevice { id: id.clone(), name: node_name.to_owned(), description, kind, node_id: Some(node_id), is_system_default, sample_rate, channels, latency };
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
    /// The raw `node.name` the "default" metadata object's `default.audio.sink`
    /// property last reported, kept even when it doesn't resolve to a known
    /// output yet. The metadata property event and the matching Sink Node
    /// global race each other at startup — confirmed live, not hypothetical:
    /// on a plain restart the metadata event can fire before the Node global
    /// that names the same sink has been discovered, so the name->OutputId
    /// lookup in the property callback finds nothing and — without this —
    /// `system_default_sink` would stay `None` for the rest of the daemon's
    /// life, wrongly leaving Signal Flow with no resolved default edge
    /// (graph.rs §13-15). `add_or_update_output` retries the lookup against
    /// this whenever a new output appears, same "remember it, retry on the
    /// event that could unblock it" shape as `pending_routes`.
    pending_default_sink_name: Option<String>,
    /// User-configurable ceiling on automatic/manual delay compensation
    /// (MTX_MULTI_OUTPUT_DELAY_COMPENSATION.md §12), independent of
    /// `ROUTE_DELAY_BUFFER_MS`'s fixed allocation size.
    max_compensation_ms: f32,
    /// Current multiplier on every non-Cue route into a cue output: 1.0
    /// normally, `Config::cue_duck_level` while the Cue group is playing.
    cue_duck_factor: f32,
    /// Consecutive ducking ticks the Cue group has been quiet — the duck
    /// releases only after a short hold so gaps between words/beats
    /// don't pump the background.
    cue_quiet_ticks: u32,
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
    /// The Master bus's own live routes (`Engine::sync_master_routes`),
    /// keyed by output like a group's `routes` — but there's exactly one
    /// master bus, not one per group, so this lives on `Engine` directly
    /// rather than inside a `GroupNode`. The paired `Vec<u32>` is the
    /// sorted group-id set this route's consumers were built against —
    /// `sync_master_routes` rebuilds a route when that set no longer
    /// matches the groups actually live, rather than just adjusting gain/
    /// enable/delay in place the way it does for every other change.
    master_routes: HashMap<OutputId, (RouteNode, Vec<u32>)>,
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
                // The removed group's taps are gone with it (dropped along
                // with its `GroupNode`) — rebuild the Master bus's consumer
                // set so it stops expecting a sample from it.
                self.sync_master_routes();
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
            Command::SyncMasterRoutes => {
                // Master going from no routes to some (or back) flips which
                // unrouted groups keep their own default route.
                self.sync_master_routes();
                let group_ids: Vec<u32> = self.groups.keys().copied().collect();
                for id in group_ids {
                    self.sync_group_routes(id);
                }
            }
            Command::SetMaxCompensationMs(ms) => {
                self.max_compensation_ms = ms.max(0.0);
                let group_ids: Vec<u32> = self.groups.keys().copied().collect();
                for id in group_ids {
                    self.sync_group_routes(id);
                }
                self.sync_master_routes();
            }
            Command::SetPreferredDefaultOutput => {
                let group_ids: Vec<u32> = self.groups.keys().copied().collect();
                for id in group_ids {
                    if let Some(node) = self.groups.get_mut(&id) {
                        node.routes.remove(&None);
                        node.route_producers.lock().unwrap().retain(|p| p.key.is_some());
                    }
                    self.sync_group_routes(id);
                }
            }
            Command::ActivateScene { scene_id } => {
                self.activate_scene(scene_id);
            }
            Command::SyncGroupPlugins { group_id } => self.sync_group_plugins(group_id),
            Command::StartRecording { id, group_id, path, format } => self.start_recording(id, group_id, path, format),
            Command::StopRecording { id } => self.stop_recording(id),
            Command::SystemResume => self.fire_event(rules::Event::SystemResume),
            Command::RebuildGroupRoutes { group_id } => {
                if let Some(node) = self.groups.get_mut(&group_id) {
                    node.routes.clear();
                    node.route_producers.lock().unwrap().clear();
                }
                self.sync_group_routes(group_id);
            }
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
                // A new group is a new voice the Master bus needs to sum in.
                self.sync_master_routes();
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
    fn sync_all_group_routes(&mut self) {
        let group_ids: Vec<u32> = self.groups.keys().copied().collect();
        for id in group_ids {
            self.sync_group_routes(id);
        }
    }

    fn sync_group_routes(&mut self, group_id: u32) {
        let (wanted, preferred_default_output, prime_threshold, via_master, feeds) = {
            let cfg = self.cfg.lock().unwrap();
            let group = cfg.groups.iter().find(|g| g.id == group_id);
            // A sub-group's only output is its parent's input (see
            // `Group::feeds_group`): one default-keyed route aimed at the
            // parent's sink node, no device routes, not via Master.
            let feeds = group.and_then(|g| g.feeds_group).filter(|p| cfg.groups.iter().any(|g| g.id == *p));
            let wanted = if feeds.is_some() { Vec::new() } else { group.map(|g| g.output_routes.clone()).unwrap_or_default() };
            // No routes of its own + a routed Master = this group plays
            // *only* through the Master bus. Keeping its own default route
            // too played it twice, so Master's fader seemed to do nothing.
            // The Cue group is never in Master and never falls back to the
            // default sink: with no cue output set it's simply silent.
            let is_cue = cfg.cue_group_id() == Some(group_id);
            let via_master = feeds.is_none() && wanted.is_empty() && (is_cue || !cfg.master_output_routes.is_empty());
            // Settings tab "buffer size" (Section 7.5): only ever consulted
            // when a *new* route is created (see `PRIME_THRESHOLD`'s own
            // doc comment in `create_route_node`) — clamped well inside
            // `RING_CAPACITY` so a bad/huge saved value can't ask for more
            // cushion than the ring buffer actually has.
            let prime_threshold =
                ((cfg.buffer_ms.clamp(10.0, 150.0) / 1000.0 * SAMPLE_RATE) as usize * CHANNELS).min(RING_CAPACITY - RING_CAPACITY / 8);
            (wanted, cfg.preferred_default_output.clone(), prime_threshold, via_master, feeds)
        };
        let Some(group_live) = self.live.lock().unwrap().get(&group_id).cloned() else { return };
        let outputs_snapshot = self.outputs.lock().unwrap().clone();
        let core = self.core.clone();
        let metrics = self.metrics.clone();
        let max_compensation_ms = self.max_compensation_ms;
        // Settings tab "audio device selection" (Section 7.5): a group with
        // no explicit output routes of its own targets this instead of bare
        // PipeWire autoconnect. Only takes effect if the preferred device
        // is actually available right now — same fail-safe-to-default
        // spirit as everywhere else in this file that resolves a device.
        //
        // Unset (or set but temporarily unplugged) falls back to
        // `output::pick_default_output`'s headphones > speakers > system-
        // default > anything policy (§5) rather than bare autoconnect —
        // deliberately *not* the default when no preference exists at all,
        // even though that's this function's real purpose: PipeWire's own
        // session manager already runs its own, more sophisticated default-
        // sink logic (profiles, Bluetooth codecs, hot-plug priority) for
        // that untouched case, and overriding it wholesale for every user
        // risks regressing behavior that already works. This only kicks in
        // once the user has expressed *some* preference and it's
        // momentarily gone — a strictly narrower, safer trigger.
        let has_preference_configured = preferred_default_output.is_some();
        let preferred_default_output_name = preferred_default_output
            .and_then(|id| outputs_snapshot.get(&id))
            .filter(|o| o.available())
            .map(|o| o.name.clone())
            .or_else(|| {
                if !has_preference_configured {
                    return None;
                }
                output::pick_default_output(outputs_snapshot.values(), self.system_default_sink.as_ref())
                    .and_then(|id| outputs_snapshot.get(&id).map(|o| o.name.clone()))
            });
        // With no hardware sink left, WirePlumber makes one of *our* group
        // sinks the system default — bare autoconnect would then feed this
        // group into MTX again (Default straight into itself: a feedback
        // loop pinned near 0 dBFS, confirmed live). So in that case target
        // real hardware explicitly, or wait with no route at all.
        let default_is_ours = self.pending_default_sink_name.as_deref().is_some_and(|n| n.starts_with("mtxpanel."));
        let preferred_default_output_name = preferred_default_output_name.or_else(|| {
            default_is_ours
                .then(|| output::pick_default_output(outputs_snapshot.values(), None))
                .flatten()
                .and_then(|id| outputs_snapshot.get(&id).map(|o| o.name.clone()))
        });
        let skip_default_route = feeds.is_none() && default_is_ours && preferred_default_output_name.is_none();
        let preferred_default_output_name = match feeds {
            Some(parent) => Some(format!("mtxpanel.group.{parent}")),
            None => preferred_default_output_name,
        };

        let wanted_keys: Vec<RouteKey> = if via_master {
            Vec::new()
        } else if wanted.is_empty() {
            vec![None]
        } else {
            wanted.iter().map(|r| Some(r.output_id.clone())).collect()
        };

        let Some(node) = self.groups.get_mut(&group_id) else { return };

        let stale: Vec<RouteKey> = node.routes.keys().filter(|k| !wanted_keys.contains(k)).cloned().collect();
        for key in &stale {
            node.routes.remove(key);
            node.route_producers.lock().unwrap().retain(|p| &p.key != key);
        }

        let mut newly_unresolved: Vec<OutputId> = Vec::new();

        if via_master {
            // Fed by the Master route's own tap instead.
        } else if wanted.is_empty() {
            if node.routes.get(&None).is_some_and(|r| skip_default_route || r.target != preferred_default_output_name) {
                node.routes.remove(&None);
                node.route_producers.lock().unwrap().retain(|p| p.key.is_some());
            }
            if skip_default_route {
                tracing::warn!(group_id, "no hardware output available; default route waits instead of looping into MTX");
            } else if !node.routes.contains_key(&None) {
                let consumer = make_tap(&node.route_producers, None);
                match create_route_node(
                    &core,
                    format!("group.{group_id}"),
                    None,
                    1.0,
                    true,
                    0.0,
                    preferred_default_output_name.clone(),
                    vec![consumer],
                    Some(group_live.clone()),
                    metrics.clone(),
                    prime_threshold,
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
                        Some(dev) => match {
                            let consumer = make_tap(&node.route_producers, key.clone());
                            create_route_node(
                                &core,
                                format!("group.{group_id}"),
                                key.clone(),
                                route_cfg.gain,
                                route_cfg.enabled,
                                delay_ms,
                                Some(dev.name.clone()),
                                vec![consumer],
                                Some(group_live.clone()),
                                metrics.clone(),
                                prime_threshold,
                            )
                        } {
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
        // This group's own routes may have just gone empty <-> non-empty,
        // which changes whether it's summed into Master. Idempotent when
        // nothing changed.
        self.sync_master_routes();
    }

    /// Reconciles the Master bus's live routes against
    /// `Config::master_output_routes` — same reconciliation shape as
    /// `sync_group_routes` above, except the source signal is every
    /// currently-live group's tap (`GroupNode::master_taps`), summed
    /// together at `create_route_node`'s own mixing point, not one group's
    /// app streams. Called whenever the group set changes (a group being
    /// created or removed changes who's being summed), whenever
    /// `master_output_routes` itself changes, and on the same output-
    /// availability events `sync_group_routes` reacts to — cheap enough at
    /// this app's scale (a handful of groups/outputs) to just fully
    /// recompute on every one of those rather than track its own pending-
    /// retry queue alongside `pending_output_routes`. Unlike a group, the
    /// Master bus has no implicit "no routes configured" fallback — it
    /// either has explicit routes or plays nowhere, since this is a purely
    /// opt-in feature nobody has to touch.
    fn sync_master_routes(&mut self) {
        let (wanted, max_compensation_ms, prime_threshold, cue_group, sub_groups) = {
            let cfg = self.cfg.lock().unwrap();
            let sub_groups: Vec<u32> = cfg.groups.iter().filter(|g| g.feeds_group.is_some()).map(|g| g.id).collect();
            let prime_threshold =
                ((cfg.buffer_ms.clamp(10.0, 150.0) / 1000.0 * SAMPLE_RATE) as usize * CHANNELS).min(RING_CAPACITY - RING_CAPACITY / 8);
            (cfg.master_output_routes.clone(), self.max_compensation_ms, prime_threshold, cfg.cue_group_id(), sub_groups)
        };
        let outputs_snapshot = self.outputs.lock().unwrap().clone();
        let core = self.core.clone();
        let metrics = self.metrics.clone();
        // Every group feeds Master — an explicit group route is an extra
        // send on top — except Cue, whose audio must stay on cue outputs,
        // and sub-groups, which reach Master through their parent.
        let mut current_group_ids: Vec<u32> =
            self.groups.keys().copied().filter(|id| Some(*id) != cue_group && !sub_groups.contains(id)).collect();
        current_group_ids.sort_unstable();

        let wanted_ids: Vec<&OutputId> = wanted.iter().map(|r| &r.output_id).collect();
        let stale: Vec<OutputId> = self.master_routes.keys().filter(|id| !wanted_ids.contains(id)).cloned().collect();
        for output_id in &stale {
            self.master_routes.remove(output_id);
            for group in self.groups.values() {
                group.master_taps.lock().unwrap().retain(|p| p.key.as_ref() != Some(output_id));
            }
        }

        for route_cfg in &wanted {
            let target = outputs_snapshot.get(&route_cfg.output_id).filter(|o| o.available());
            let delay_ms = output::resolve_route_delay_ms(route_cfg, &wanted, &outputs_snapshot, max_compensation_ms);

            // A master route's consumers are fixed to whichever groups
            // existed when it was (re)built — unlike a plain group route,
            // where the *group's own* shape never changes underneath an
            // already-live route, the master's input set does, every time
            // a group is created or removed. Force a rebuild for that case
            // alongside the existing "device went away" one, instead of
            // only ever adjusting gain/enable/delay in place — otherwise a
            // newly created group's audio would silently never join the
            // mix until some unrelated future event happened to rebuild it.
            let needs_rebuild = match self.master_routes.get(&route_cfg.output_id) {
                Some((_, built_for)) => target.is_none() || built_for != &current_group_ids,
                None => false,
            };
            if needs_rebuild {
                self.master_routes.remove(&route_cfg.output_id);
                for group in self.groups.values() {
                    group.master_taps.lock().unwrap().retain(|p| p.key.as_ref() != Some(&route_cfg.output_id));
                }
            }

            match self.master_routes.get(&route_cfg.output_id) {
                Some((existing, _)) => {
                    existing.gain.set_target(route_cfg.gain);
                    existing.enabled.store(route_cfg.enabled, Ordering::Relaxed);
                    existing.delay.lock().unwrap().set_delay_ms(delay_ms, SAMPLE_RATE);
                }
                None => {
                    if let Some(dev) = target {
                        let key: RouteKey = Some(route_cfg.output_id.clone());
                        let consumers: Vec<rtrb::Consumer<f32>> =
                            current_group_ids.iter().filter_map(|id| self.groups.get(id)).map(|g| make_tap(&g.master_taps, key.clone())).collect();
                        match create_route_node(
                            &core,
                            "master".to_owned(),
                            key,
                            route_cfg.gain,
                            route_cfg.enabled,
                            delay_ms,
                            Some(dev.name.clone()),
                            consumers,
                            None,
                            metrics.clone(),
                            prime_threshold,
                        ) {
                            Ok(route) => {
                                self.master_routes.insert(route_cfg.output_id.clone(), (route, current_group_ids.clone()));
                            }
                            Err(err) => tracing::error!(%err, output_id = %route_cfg.output_id, "failed to create master route"),
                        }
                    }
                }
            }
        }
        // Syncs reset route gains to configured; re-apply an active duck.
        if self.cue_duck_factor != 1.0 {
            self.apply_cue_duck();
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
        // The Master bus keeps no pending-retry queue of its own (see
        // `sync_master_routes`'s doc comment) — just recheck it fresh
        // whenever any output's availability changes.
        self.sync_master_routes();
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
    /// The "✕ remove from group" UI action: takes a stream out of whatever
    /// *custom* group it's in and back into the protected "Default" group —
    /// never out of the mixer entirely. Confirmed live as a real bug when
    /// this used to reconnect straight to hardware instead: every stream
    /// shown in the Inputs tab is supposed to always be inside some group
    /// (Default at minimum, per this whole app's own stated design — every
    /// Inputs row already assumes its stream is being processed and mixed
    /// somewhere), but bypassing to raw hardware silently violated that,
    /// leaving a stream invisible to MTX Panel/Faders until the user
    /// happened to notice and manually re-route it. `shutdown_gracefully`
    /// (an entirely different situation — the daemon is about to exit, so
    /// routing into Default's own about-to-vanish virtual sink would
    /// achieve nothing) uses the separate `reconnect_to_hardware` below
    /// instead of this method.
    fn unroute_stream(&mut self, stream_id: u32) {
        let Some(default_group_id) = self.cfg.lock().unwrap().groups.iter().find(|g| g.protected).map(|g| g.id) else {
            tracing::warn!(stream_id, "unroute_stream: no protected default group exists");
            return;
        };
        self.route_and_persist(stream_id, default_group_id);
    }

    /// Reconnects `stream_id` straight to whatever real hardware sink is
    /// currently in use — the graceful-shutdown-only path (see
    /// `shutdown_gracefully` below); a manual "remove from group" now goes
    /// through `unroute_stream` above instead, which returns the stream to
    /// Default rather than bypassing the mixer.
    fn reconnect_to_hardware(&mut self, stream_id: u32) {
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
                tracing::info!(stream_id, hw_node, "reconnected stream to hardware");
            }
            None => tracing::warn!(stream_id, "tried to reconnect to hardware, but no hardware sink found"),
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
            self.reconnect_to_hardware(stream_id);
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
        let mut retired_id: Option<OutputId> = None;
        if let Some(node_id) = node_id {
            if let Some((stale_id, _)) = outputs.iter().find(|(existing_id, o)| o.node_id == Some(node_id) && *existing_id != &id) {
                retired_id = Some(stale_id.clone());
            }
            outputs.retain(|existing_id, o| o.node_id != Some(node_id) || existing_id == &id);
        }
        // A *disconnect-then-reconnect* (not the same-session reclassification
        // above) can also land under a different id than last time: the same
        // physical output can resolve its identity differently across
        // separate PipeWire Device announcements — confirmed live, e.g. a
        // machine's built-in card reappearing without `device.vendor.id`/
        // `device.product.id` populated yet, falling back to the `name:`
        // tier instead of `pci:...` again. `node_id` can't catch this (the
        // old entry's is already `None` from `mark_output_unavailable`), so
        // fall back to matching the one property that *is* still the same
        // physical sink either way: the underlying PipeWire `node.name`.
        // Deliberately only merges against a currently-*unavailable* entry —
        // two genuinely distinct live outputs sharing a name would be a real
        // (if unlikely) setup this must not silently collapse into one.
        if retired_id.is_none() {
            if let Some((stale_id, _)) = outputs.iter().find(|(existing_id, o)| !o.available() && o.name == output.name && *existing_id != &id) {
                retired_id = Some(stale_id.clone());
            }
        }
        if let Some(retired_id) = &retired_id {
            outputs.remove(retired_id);
        }
        // The id just retired above was the resolved default for this same
        // physical device — carry the flag over to its new id rather than
        // leaving `system_default_sink` pointing at an id that no longer
        // exists in `outputs` (which would silently un-resolve Signal
        // Flow's default edge until the next real default-sink change
        // fires another metadata event).
        if retired_id.is_some() && self.system_default_sink.as_ref() == retired_id.as_ref() {
            self.system_default_sink = Some(id.clone());
        }
        outputs.insert(id.clone(), output);
        drop(outputs);
        // Retry resolving the system default against this newly-arrived
        // output — see `pending_default_sink_name`'s doc comment for why
        // this can't just be resolved once in the metadata property
        // callback and forgotten.
        if self.system_default_sink.is_none() {
            if let Some(name) = &self.pending_default_sink_name {
                let outputs = self.outputs.lock().unwrap();
                if let Some(matched) = outputs.values().find(|o| &o.name == name).map(|o| o.id.clone()) {
                    self.system_default_sink = Some(matched);
                }
            }
        }
        self.sync_system_default_flags();
        self.retry_pending_output_routes(&id);
        // A default route may have been waiting on (or explicitly
        // targeting) hardware — see `sync_group_routes`.
        self.sync_all_group_routes();
        if !was_available {
            self.fire_event(rules::Event::DeviceConnected { output_id: id });
        }
    }

    /// Keeps every `OutputDevice.is_system_default` flag in `outputs` in
    /// sync with `system_default_sink` — must be called any time either
    /// changes, not just once at output-discovery time, or `ListOutputs`/
    /// Signal Flow (graph.rs) would keep reporting a default sink the user
    /// switched away from (e.g. via pavucontrol) while the daemon was
    /// already running.
    fn sync_system_default_flags(&mut self) {
        let mut outputs = self.outputs.lock().unwrap();
        for (id, output) in outputs.iter_mut() {
            output.is_system_default = self.system_default_sink.as_ref() == Some(id);
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
        // Covers both device routes to this output and default routes
        // explicitly targeting it (see `sync_group_routes`).
        self.sync_all_group_routes();
        self.sync_master_routes();
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
    fn activate_scene(&mut self, scene_id: u32) {
        let Some(scene) = self.cfg.lock().unwrap().scenes.iter().find(|s| s.id == scene_id).cloned() else {
            tracing::warn!(scene_id, "activate_scene: no such scene");
            return;
        };
        let (mut applied_groups, mut missing_groups) = (0, 0);

        self.master_gain.set_target(scene.master_gain);
        {
            let mut cfg = self.cfg.lock().unwrap();
            cfg.master_gain = scene.master_gain;
            let _ = cfg.save();
        }

        for snapshot in &scene.groups {
            let exists = self.cfg.lock().unwrap().groups.iter().any(|g| g.id == snapshot.id);
            if !exists {
                missing_groups += 1;
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
                    // Cue routing belongs to cue_outputs, not to scenes.
                    if !g.cue {
                        g.output_routes = snapshot.output_routes.clone();
                    }
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
            applied_groups += 1;
        }

        // Exact per-stream placements first, then app-level fallbacks for
        // streams that didn't exist at capture (see `Scene`'s own docs).
        let live: Vec<(u32, AppId)> = self.streams.lock().unwrap().iter().map(|(id, s)| (*id, s.app_id.clone())).collect();
        let (moves, pending) = crate::scenes::plan_stream_moves(&scene, &live);
        for (stream_id, group_id) in moves {
            self.route_and_persist(stream_id, group_id);
        }
        let pending_streams = pending.len();
        for (app_id, group_id) in pending {
            self.active_scene_pending.insert(app_id, group_id);
        }

        tracing::info!(
            scene_id,
            name = scene.name,
            applied_groups,
            missing_groups,
            pending_streams,
            "scene activated"
        );
        self.fire_event(rules::Event::SceneActivated { scene_id });
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
                actions_summary: rules::describe_actions(&rule.actions, &self.cfg.lock().unwrap().groups),
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
                        rules::Event::AppStarted { app_id, stream_id, .. } => (app_id.clone(), *stream_id),
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
        self.evaluate_cue_duck();
        let rule_list = self.cfg.lock().unwrap().ducking_rules.clone();
        if rule_list.is_empty() {
            return;
        }
        let targets = {
            let live = self.live.lock().unwrap();
            self.duck_engine.evaluate(
                &rule_list,
                // Pre-fader: the trigger's own fader must not decide whether
                // others duck (it did — raising Classes' fader ducked the
                // song, lowering it un-ducked it).
                |group_id| live.get(&group_id).map(|l| l.input_peak()),
                tick_seconds,
            )
        };
        for (group_id, gain) in targets {
            if let Some(node) = self.groups.get(&group_id) {
                node.params.duck_gain.set_target(gain);
            }
        }
    }

    /// Cue ducking: while the Cue group is audible, everything else on the
    /// cue outputs (Master's and other groups' routes into them) drops to
    /// `cue_duck_level`, so the cued audio stands out in the headphones.
    /// Other outputs (the speakers) are never touched.
    fn evaluate_cue_duck(&mut self) {
        const CUE_ACTIVE_LEVEL: f32 = 0.001; // ~-60 dBFS
        const CUE_RELEASE_TICKS: u32 = 25; // ~500ms at the 20ms tick
        let (cue_id, duck_level) = {
            let cfg = self.cfg.lock().unwrap();
            // No cue output = cue mode off: never duck, whatever sits in Cue.
            let cue_id = if cfg.cue_outputs.is_empty() { None } else { cfg.cue_group_id() };
            (cue_id, cfg.cue_duck_level)
        };
        let level = cue_id
            .and_then(|id| self.live.lock().unwrap().get(&id).map(|l| l.input_peak()))
            .unwrap_or(0.0);
        if level > CUE_ACTIVE_LEVEL {
            self.cue_quiet_ticks = 0;
        } else {
            self.cue_quiet_ticks = self.cue_quiet_ticks.saturating_add(1);
        }
        let factor = if self.cue_quiet_ticks < CUE_RELEASE_TICKS { duck_level } else { 1.0 };
        if factor != self.cue_duck_factor {
            tracing::info!(factor, "cue duck {}", if factor < 1.0 { "engaged" } else { "released" });
            self.cue_duck_factor = factor;
            self.apply_cue_duck();
        }
    }

    /// Re-targets every route gain into a cue output as configured gain ×
    /// `cue_duck_factor` (other outputs get plain configured gain). Also
    /// called after any route sync, which resets gains to configured.
    fn apply_cue_duck(&self) {
        let cfg = self.cfg.lock().unwrap();
        let duck = |output_id: &OutputId| if cfg.cue_outputs.contains(output_id) { self.cue_duck_factor } else { 1.0 };
        for r in &cfg.master_output_routes {
            if let Some((route, _)) = self.master_routes.get(&r.output_id) {
                route.gain.set_target(r.gain * duck(&r.output_id));
            }
        }
        let cue_id = cfg.cue_group_id();
        for g in cfg.groups.iter().filter(|g| Some(g.id) != cue_id) {
            let Some(node) = self.groups.get(&g.id) else { continue };
            for r in &g.output_routes {
                if let Some(route) = node.routes.get(&Some(r.output_id.clone())) {
                    route.gain.set_target(r.gain * duck(&r.output_id));
                }
            }
            // A group with no routes (and no routed Master) plays through
            // its implicit default route (unity gain) to the system
            // default output — duck it too when that output is a cue output.
            if let Some(route) = node.routes.get(&None) {
                let factor = self.system_default_sink.as_ref().map_or(1.0, |id| duck(id));
                route.gain.set_target(factor);
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
        let mut chain = node.plugin_chain.lock().unwrap();
        if !chain.update_params(&instances) {
            *chain = plugins::PluginChain::build(&instances, SAMPLE_RATE);
        }
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
        match crate::recording::start(group_id, path.clone(), format) {
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
///
/// Run off-thread: wpctl can wait on PipeWire, which can wait on this very
/// engine thread — blocking here on it is a self-deadlock.
fn set_stream_volume(stream_id: u32, linear: f32) {
    let volume = format!("{:.4}", linear.max(0.0));
    std::thread::spawn(move || {
        let _ = std::process::Command::new("timeout").args(["2", "wpctl", "set-volume", &stream_id.to_string(), &volume]).status();
    });
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
    let master_taps: Arc<Mutex<Vec<RouteProducer>>> = Arc::new(Mutex::new(Vec::new()));

    // Per-band/per-channel filter state lives only in this closure's capture
    // — it's mutated every sample but never touched from another thread.
    let band_count = params.eq_bands.len();
    let mut eq_state: Vec<[Biquad; CHANNELS]> = vec![[Biquad::default(); CHANNELS]; band_count];
    let mut hp_state = [Biquad::default(); CHANNELS];
    let mut lp_state = [Biquad::default(); CHANNELS];
    let mut dynamics = DynamicsProcessor::new();
    // Always-on output safety net (not a user setting) — see `Limiter`'s
    // own doc comment for why this exists and what it doesn't replace
    // (the hard `clamp` a few lines below it in the frame loop).
    let mut limiter = Limiter::new();
    const LIMITER_CEILING_DB: f32 = -0.3;
    const LIMITER_ATTACK_MS: f32 = 1.0;
    const LIMITER_RELEASE_MS: f32 = 50.0;
    // Fixed constants, never user-set — computed once here instead of via
    // `exp()` on every single sample (this plus the compressor/gate's own
    // attack/release coefficients recomputing the same way was a real,
    // measured chunk of the audio thread's steady-state CPU use).
    let limiter_attack_coeff = crate::dsp::dynamics::one_pole_coeff(LIMITER_ATTACK_MS, SAMPLE_RATE);
    let limiter_release_coeff = crate::dsp::dynamics::one_pole_coeff(LIMITER_RELEASE_MS, SAMPLE_RATE);
    // recompute_every=4: an FFT every callback (~10ms) is wasted work no
    // 200ms-polled UI could ever show; ~40ms cadence still reads as live.
    let mut spectrum = SpectrumAnalyzer::new(SAMPLE_RATE, 4);

    let capture_params = params.clone();
    let capture_metrics = metrics.clone();
    let capture_route_producers = route_producers.clone();
    let capture_plugin_chain = plugin_chain.clone();
    let capture_recording_producers = recording_producers.clone();
    let capture_master_taps = master_taps.clone();
    let capture_listener = capture
        .add_local_listener_with_user_data(())
        .process(move |stream, _| {
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }
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
            if n_frames > 0 {
                live.record_capture_tick();
            }
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

            // Same "rebuild once per buffer, not per sample" rule as the EQ
            // coefficients just above — these are `exp()` calls, and the
            // compressor/gate's attack/release times ramp too slowly for a
            // per-sample rebuild to matter. `.next()` still runs on the
            // underlying smoothers every sample below, so they keep ticking.
            let comp_attack_coeff = crate::dsp::dynamics::one_pole_coeff(capture_params.compressor.attack.current(), SAMPLE_RATE);
            let comp_release_coeff = crate::dsp::dynamics::one_pole_coeff(capture_params.compressor.release.current(), SAMPLE_RATE);
            let gate_attack_coeff = crate::dsp::dynamics::one_pole_coeff(capture_params.gate.attack.current(), SAMPLE_RATE);
            let gate_release_coeff = crate::dsp::dynamics::one_pole_coeff(capture_params.gate.release.current(), SAMPLE_RATE);

            let mut buf_peak = [0f32; CHANNELS];
            let mut in_peak = 0f32;
            let mut producers = capture_route_producers.lock().unwrap();
            let mut recordings = capture_recording_producers.lock().unwrap();
            let mut master_taps = capture_master_taps.lock().unwrap();
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
                    in_peak = in_peak.max(raw.abs());
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
                    let out = dynamics.process(peak.max(1e-9), &comp, comp_attack_coeff, comp_release_coeff, &gate, gate_attack_coeff, gate_release_coeff, SAMPLE_RATE);
                    out / peak.max(1e-9)
                };

                // Plugin chain slots in here (NewImprovenments/
                // 10-plugin-dsp-architecture.md's `Gain -> EQ -> Filter ->
                // Compressor -> Gate -> Plugin(s) -> Output`) — after every
                // built-in stage, before the signal fans out to routes and
                // recordings, so both hear exactly what a plugin did to it.
                let mut frame = [samples[0] * mult, samples[1] * mult];
                plugin_chain.process(&mut frame);

                // Always-on safety limiter — nothing upstream (EQ boosts, a
                // compressor's manual makeup gain, a plugin) is required to
                // keep the signal inside [-1, 1] on its own, and none of
                // them individually know what the *others* already added.
                // Live-confirmed as a real, currently-happening bug, not a
                // theoretical one: a real group's actual settings (6
                // boosted EQ bands + a compressor with +6.14dB makeup) work
                // out to modeled output *over* 0dBFS for any input that
                // reaches the compressor at 0dBFS or louder, growing worse
                // the louder the source gets — audible as real distortion
                // on loud passages, reported live as "audio isn't clear in
                // some parts." Stereo-linked (one gain from the louder
                // channel's peak, applied to both) so a loud transient
                // can't shift the stereo image. Doesn't replace the
                // optional `soft_clipper` plugin (that's deliberate,
                // musical saturation someone opts into) — this is the
                // always-on floor underneath it.
                let frame_peak = frame[0].abs().max(frame[1].abs());
                let limiter_gain = limiter.process(frame_peak, LIMITER_CEILING_DB, limiter_attack_coeff, limiter_release_coeff);
                frame[0] *= limiter_gain;
                frame[1] *= limiter_gain;
                // Backstop for the sliver a zero-lookahead limiter can't
                // fully catch on a truly instantaneous step — see
                // `Limiter`'s own doc comment.
                frame[0] = frame[0].clamp(-1.0, 1.0);
                frame[1] = frame[1].clamp(-1.0, 1.0);

                for (ch, out) in frame.into_iter().enumerate() {
                    buf_peak[ch] = buf_peak[ch].max(out.abs());
                    for route in producers.iter_mut() {
                        let _ = route.producer.push(out);
                    }
                    for rec in recordings.iter_mut() {
                        let _ = rec.producer.push(out);
                    }
                    for tap in master_taps.iter_mut() {
                        let _ = tap.producer.push(out);
                    }
                }
            }
            drop(producers);
            drop(recordings);
            drop(master_taps);
            live.publish(buf_peak[0], buf_peak[1], dynamics.gain_reduction_db(), dynamics.gate_reduction_db());
            live.publish_input_peak(in_peak);
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
        master_taps,
        _capture_listener: capture_listener,
    })
}

/// Creates one ring-buffer tap into `producers` and returns its consumer
/// half for whichever route node reads it. Retains any stale entry under
/// the same `key` first — a route re-created after its device disappeared
/// and came back must not leave the old, now-consumerless producer sitting
/// in the fan-out list.
fn make_tap(producers: &Arc<Mutex<Vec<RouteProducer>>>, key: RouteKey) -> rtrb::Consumer<f32> {
    let mut guard = producers.lock().unwrap();
    guard.retain(|p| p.key != key);
    let (tx, rx) = rtrb::RingBuffer::<f32>::new(RING_CAPACITY);
    guard.push(RouteProducer { key, producer: tx });
    rx
}

/// Fullest consumer's occupancy. Not the emptiest: a Master route sums
/// every group, and an idle group (no streams, so PipeWire suspends its
/// capture) never fills its tap — gating on min kept Master silent
/// forever. Empty consumers just contribute silence in the pop loop. A
/// plain group route has one consumer, so max == min there.
// ponytail: one shared cushion for all Master inputs; a group that starts
// playing while Master is already primed gets no cushion of its own (may
// underrun briefly). Per-consumer priming if that's ever audible.
fn max_slots(consumers: &[rtrb::Consumer<f32>]) -> usize {
    consumers.iter().map(|c| c.slots()).max().unwrap_or(0)
}

/// Builds one route's playback stream: its own gain/delay/enable state
/// (MTX_AUDIO_GROUPS_MULTI_OUTPUT.md §7/§8) feeding real output hardware,
/// summing one or more ring-buffer consumers (`make_tap`'s consumer half)
/// each frame. A plain group route always has exactly one — the group's
/// own post-DSP signal, unchanged behavior from before this was
/// generalized. The Master bus route (`Engine::sync_master_routes`) has
/// one per currently-live group instead, so *this* closure is the actual
/// mixing point that sums them into the final bus, reusing every bit of
/// this function's existing priming/delay/gain machinery rather than a
/// second hand-written playback stream.
///
/// `live`: `None` for a Master route. Underrun accounting needs one
/// group's liveness signal to know "should a consumer have data right
/// now", and there's no single such group for a sum of several — skipping
/// the count there is simpler than inventing a many-groups liveness
/// concept for what's ultimately just a metrics nicety.
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
    stream_label: String,
    key: RouteKey,
    initial_gain: f32,
    initial_enabled: bool,
    initial_delay_ms: f32,
    target_node_name: Option<String>,
    mut consumers: Vec<rtrb::Consumer<f32>>,
    live: Option<Arc<GroupLive>>,
    metrics: Arc<Metrics>,
    prime_threshold: usize,
) -> Result<RouteNode, pw::Error> {
    let route_label = key.clone().unwrap_or_else(|| "default".to_owned());
    let stream_name = format!("mtxpanel.{stream_label}.route.{route_label}");

    let mut props = properties! {
        *keys::MEDIA_TYPE => "Audio",
        *keys::MEDIA_CATEGORY => "Playback",
        *keys::MEDIA_ROLE => "Music",
        *keys::NODE_NAME => stream_name.as_str(),
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

    let route_gain = gain.clone();
    let route_enabled = enabled.clone();
    let route_delay = delay.clone();
    let route_metrics = metrics.clone();
    let route_live = live.clone();
    // Grace window for the capture-liveness check below: capture and this
    // playback stream aren't linked to each other in the PipeWire graph (no
    // real port connection exists between them, just our own ring buffer),
    // so there's no ordering guarantee that capture's cycle for a given
    // quantum lands before this one's — a couple of stale cycles is normal
    // jitter, not a real gap. Small enough to still catch a genuine glitch
    // within ~30ms, generous enough not to flag routine scheduling order.
    const CAPTURE_STALE_GRACE_CYCLES: u32 = 3;
    let mut last_capture_tick: u64 = 0;
    let mut stale_cycles: u32 = CAPTURE_STALE_GRACE_CYCLES;
    // Without an explicit warm-up, this closure starts popping the instant
    // it's created, racing whatever capture has managed to push by then —
    // it settles at whatever thin margin that race happens to leave (live-
    // measured: ~16ms of the ring's ~170ms capacity), and capture/playback
    // aren't linked in the graph (see the grace-window comment above), so
    // an occasional single-cycle scheduling-order flip can fully drain a
    // margin that thin — live-verified as rare, isolated, one-quantum
    // (1024-sample) misses during otherwise-clean playback. Withholding
    // real output until a much larger cushion has built up first fixes the
    // cause (too little slack), not just the symptom.
    // `prime_threshold` (Settings tab "buffer size", Section 7.5): the
    // caller (`sync_group_routes`) already clamped this well inside
    // `RING_CAPACITY`.
    //
    // `primed` re-arms itself once occupancy drains below `low_watermark`,
    // rather than latching true forever after the first fill — confirmed
    // live as a real, not just theoretical, gap: a one-shot latch is fine
    // for the *startup* race this cushion exists for, but every ordinary
    // silence (a paused video, a YouTube ad, switching clips) drains the
    // ring buffer to genuinely empty through this same pop loop regardless
    // of `primed`, and with no re-arm, playback resumed real content with
    // *zero* slack from then on — turning every subsequent ordinary
    // scheduling jitter into a real, counted underrun instead of one it
    // could absorb. Live-verified: a daemon left running for ~4.5 hours of
    // normal YouTube/browser use (i.e. plenty of pauses/ad breaks) climbed
    // to 5.2M+ underruns at a sustained ~60k/sec — not the rare isolated
    // misses this cushion was built to catch, but exactly this permanent-
    // zero-slack state, self-inflicted the first time real content resumed
    // after any silence. Re-arming at a low watermark (rather than only
    // once, at creation) means a resume pays a one-time ~prime_threshold
    // silent delay to rebuild slack instead of running the rest of the
    // session on none.
    let low_watermark = prime_threshold / 4;
    let mut primed = false;
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

            // Is capture *currently* relaying anything, at all, right now?
            // See `GroupLive`'s own doc comment for why this (not "is a
            // stream routed" or "is it `NodeState::Running`") is the only
            // honest signal for "should this ring buffer have data" —
            // popping empty when capture itself has nothing to give isn't
            // a glitch, it's silence. `None` (a Master route) skips this
            // entirely — see this function's own doc comment on `live`.
            let capture_alive = match &route_live {
                Some(live) => {
                    let current_tick = live.capture_ticks();
                    if current_tick != last_capture_tick {
                        stale_cycles = 0;
                        last_capture_tick = current_tick;
                    } else {
                        stale_cycles = stale_cycles.saturating_add(1);
                    }
                    stale_cycles < CAPTURE_STALE_GRACE_CYCLES
                }
                None => false,
            };

            if primed && max_slots(&consumers) < low_watermark {
                primed = false;
            }
            if !primed {
                if max_slots(&consumers) >= prime_threshold {
                    primed = true;
                } else {
                    // Still building the cushion — silence, not an
                    // underrun: nothing has actually failed to arrive yet,
                    // we're deliberately not asking for it.
                    for frame in 0..n_frames {
                        for ch in 0..CHANNELS {
                            let start = frame * stride + ch * 4;
                            slice[start..start + 4].copy_from_slice(&0f32.to_le_bytes());
                        }
                    }
                    let chunk = data.chunk_mut();
                    *chunk.offset_mut() = 0;
                    *chunk.stride_mut() = stride as _;
                    *chunk.size_mut() = (stride * n_frames) as _;
                    return;
                }
            }

            let is_enabled = route_enabled.load(Ordering::Relaxed);
            let mut delay_lines = route_delay.lock().unwrap();
            for frame in 0..n_frames {
                let g = route_gain.next();
                for ch in 0..CHANNELS {
                    // A plain group route always has exactly one consumer,
                    // so this is exactly the old single-`pop` behavior; a
                    // Master route sums every tapped group's sample for
                    // this channel/frame into the bus. One or more empty
                    // consumers this cycle only counts as an underrun while
                    // capture is currently alive — see above.
                    let mut sample = 0.0f32;
                    let mut any_empty = false;
                    for consumer in consumers.iter_mut() {
                        match consumer.pop() {
                            Ok(v) => sample += v,
                            Err(_) => any_empty = true,
                        }
                    }
                    if any_empty && capture_alive {
                        route_metrics.underruns.fetch_add(1, Ordering::Relaxed);
                    }
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

    Ok(RouteNode { playback, gain, enabled, delay, target: target_node_name, _listener: listener })
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
        pending_default_sink_name: None,
        // Re-clamped on load too, not just in dbus.rs's setter — defends
        // against a config file written before that clamp existed, or
        // hand-edited past the real buffer limit.
        max_compensation_ms: cfg.max_compensation_ms.clamp(0.0, ROUTE_DELAY_BUFFER_MS),
        cue_duck_factor: 1.0,
        cue_quiet_ticks: u32::MAX, // start released, not ducked
        streams: streams.clone(),
        rule_runtime: rules::RuleRuntime::default(),
        active_scene_pending: HashMap::new(),
        duck_engine: ducking::DuckingEngine::default(),
        recordings: HashMap::new(),
        recording_status,
        master_routes: HashMap::new(),
    }));

    // Parents before sub-groups, so a sub-group's target node exists.
    let mut startup_groups: Vec<&Group> = cfg.groups.iter().collect();
    startup_groups.sort_by_key(|g| g.feeds_group.is_some());
    for group in startup_groups {
        engine.borrow_mut().create_group(group);
    }
    engine.borrow_mut().sync_master_routes();

    let streams_for_registry = streams.clone();
    let streams_for_prune = streams.clone();
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
    // uses `reconnect_to_hardware` — a link-to-real-hardware path kept
    // separate from `unroute_stream`'s manual "remove from group" action,
    // which now returns a stream to the protected Default group instead of
    // bypassing the mixer (see both methods' own doc comments for why).
    // Async link creation needs the same care either way: quitting
    // immediately after requesting it, as a first version of this did, let
    // the process exit before the request ever reached the server, silently
    // dropping it despite `object.linger` (also observed live: the log claimed success,
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

    // systemd watchdog (WatchdogSec in the unit): pinged from this
    // mainloop, so if the engine thread ever wedges (the output-solo
    // deadlock did exactly that, silently, with the process still alive)
    // the pings stop and systemd kills and restarts the daemon.
    let watchdog_timer = mainloop.loop_().add_timer(|_| crate::sd_notify("WATCHDOG=1"));
    watchdog_timer.update_timer(Some(Duration::from_secs(2)), Some(Duration::from_secs(2)));
    crate::sd_notify("READY=1");

    // global_remove only prunes streams that close while we're watching;
    // ones that closed while the daemon was down (or wedged) linger in
    // routed_streams as dead "stream #N" rows — and PipeWire reuses ids, so
    // a stale entry would later swallow an unrelated new stream. The first
    // `done` after this sync means the registry has replayed every live
    // global, so anything not in `streams` by then is gone for good.
    let engine_for_prune = engine.clone();
    let pruned = std::cell::Cell::new(false);
    let _prune_listener = core
        .add_listener_local()
        .done(move |_id, _seq| {
            if pruned.replace(true) {
                return;
            }
            let live = streams_for_prune.lock().unwrap();
            let engine = engine_for_prune.borrow();
            let mut cfg = engine.cfg.lock().unwrap();
            let mut changed = false;
            for group in cfg.groups.iter_mut() {
                let before = group.routed_streams.len();
                group.routed_streams.retain(|sid| live.contains_key(sid));
                changed |= group.routed_streams.len() != before;
            }
            if changed {
                tracing::info!("pruned stream ids that closed while the daemon wasn't running");
                let _ = cfg.save();
            }
        })
        .register();
    let _ = core.sync(0);

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
        let mut engine = engine.borrow_mut();
        engine.links.insert(global.id, LinkInfo { output_node, input_node });
        // PipeWire's own autoconnect can relink an already-routed stream
        // straight back to hardware behind our back with no Port event
        // (the ports already exist) — `retry_pending_routes_for_node` never
        // fires to catch it. If this brand-new link's source is a stream
        // we've already routed into a group, and it doesn't feed that
        // group's capture node, it's exactly the unwanted parallel copy
        // `sever_existing_links` exists to prevent — kill it immediately.
        if let Some(&group_id) = engine.stream_group.get(&output_node) {
            let capture_id = engine.groups.get(&group_id).map(|g| g.capture.node_id());
            if capture_id != Some(input_node) {
                match engine.registry.destroy_global(global.id).into_result() {
                    Ok(_) => tracing::info!(output_node, input_node, group_id, "severed stray autoconnect link to hardware"),
                    Err(err) => tracing::warn!(%err, link_id = global.id, "failed to remove stray autoconnect link"),
                }
            }
        }
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
                    engine.system_default_sink = name.as_ref().and_then(|name| {
                        engine.outputs.lock().unwrap().values().find(|o| &o.name == name).map(|o| o.id.clone())
                    });
                    // Kept even when the lookup above just failed — the
                    // matching Sink Node global may simply not have arrived
                    // yet; `add_or_update_output` retries against this name
                    // once it does (see `pending_default_sink_name`'s doc).
                    engine.pending_default_sink_name = name;
                    engine.sync_system_default_flags();
                    engine.sync_all_group_routes();
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
    streams.lock().unwrap().insert(global.id, AppStream { id: global.id, name, service: service.clone(), running: false, app_id: app_id.clone() });
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
            engine.borrow_mut().fire_event(rules::Event::AppStarted {
                app_id: app_id.clone(),
                stream_id: global.id,
                service: service.clone(),
            });
        }
    }
    // Only a genuinely-new arrival gets a second AppStarted firing below
    // once its browser tab title resolves — a daemon-restart replay of an
    // already-known stream must not re-trigger automation (see the doc
    // comment on the match above).
    let is_new_arrival = existing_group.is_none();

    // A name alone doesn't tell "is this one actually making sound right
    // now" apart from another instance of the same app (Section 7.2, and
    // `service` above can still collide — two tabs on the same site). Bind
    // the node and track its RUNNING state rather than guessing from a name.
    let stream_id = global.id;
    let registry = engine.borrow().registry.clone();
    match registry.bind::<pw::node::Node, _>(global) {
        Ok(node) => {
            let streams_for_info = streams.clone();
            let engine_for_info = engine.clone();
            let app_id_for_info = app_id.clone();
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
                    let mut newly_resolved = None;
                    if let Some(s) = streams_for_info.lock().unwrap().get_mut(&stream_id) {
                        s.running = running;
                        if let Some(service) = service.filter(|svc| svc != &s.name) {
                            if is_new_arrival && s.service != service {
                                newly_resolved = Some(service.clone());
                            }
                            s.service = service;
                        }
                    }
                    // A site-filtered rule (e.g. "route Udemy tabs to group
                    // 6") can't match at stream-creation time — the tab
                    // title isn't known yet, see the doc comment above — so
                    // re-evaluate rules now that it is. `fire_event`'s own
                    // per-rule cooldown keeps this from re-routing on every
                    // later title change (new video, tab navigation) more
                    // than once in a burst.
                    if let Some(service) = newly_resolved {
                        engine_for_info.borrow_mut().fire_event(rules::Event::AppStarted {
                            app_id: app_id_for_info.clone(),
                            stream_id,
                            service,
                        });
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

        assert_eq!(live.capture_ticks(), 0);
        live.record_capture_tick();
        live.record_capture_tick();
        assert_eq!(live.capture_ticks(), 2);
    }

    /// Direct regression test for the multi-tab bug found live: a scene (or
    /// profile) resolving "the stream for this app" must return *every*
    /// live stream sharing that identity, not just one — see
    /// `streams_matching_app_id`'s own doc comment for the full story.
    #[test]
    fn streams_matching_app_id_returns_every_match_not_just_one() {
        let mut streams = HashMap::new();
        streams.insert(84, AppStream { id: 84, name: "Firefox".into(), service: "WhatsApp".into(), running: true, app_id: "appname:Firefox".into() });
        streams.insert(85, AppStream { id: 85, name: "Firefox".into(), service: "YouTube".into(), running: true, app_id: "appname:Firefox".into() });
        streams.insert(120, AppStream { id: 120, name: "Firefox".into(), service: "YouTube".into(), running: true, app_id: "appname:Firefox".into() });
        streams.insert(200, AppStream { id: 200, name: "Spotify".into(), service: String::new(), running: true, app_id: "appname:Spotify".into() });

        let mut firefox_ids = streams_matching_app_id(&streams, &"appname:Firefox".to_owned());
        firefox_ids.sort_unstable();
        assert_eq!(firefox_ids, vec![84, 85, 120], "all three Firefox tabs must be returned, not just the first match");

        assert_eq!(streams_matching_app_id(&streams, &"appname:Spotify".to_owned()), vec![200]);
        assert!(streams_matching_app_id(&streams, &"appname:Nonexistent".to_owned()).is_empty());
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
        // Real Udemy tab title (live-verified) — no " - " anywhere, only
        // " | ", which used to fall through and return the whole title.
        assert_eq!(
            extract_service_name("Course: Kubernetes Certified Application Developer (CKAD) with Tests | Udemy"),
            Some("Udemy".to_owned())
        );
        // A title using both separators still ends on the real site name.
        assert_eq!(extract_service_name("Chapter 1 - Intro | Udemy"), Some("Udemy".to_owned()));
    }

    #[test]
    fn extract_json_name_pulls_the_name_field() {
        assert_eq!(extract_json_name(r#"{"name":"alsa_output.pci-0000_00_1f.3"}"#), Some("alsa_output.pci-0000_00_1f.3".to_owned()));
        assert_eq!(extract_json_name(r#"{"name": "bluez_output.AA_BB"}"#), Some("bluez_output.AA_BB".to_owned()));
        assert_eq!(extract_json_name("null"), None);
    }
}
