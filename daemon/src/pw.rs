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
use std::sync::atomic::{AtomicBool, Ordering};
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
}

/// Control-thread -> pw-thread messages (PROJECT_SPEC.md Section 8's D-Bus
/// methods, minus the read-only ones which just read shared state directly).
pub enum Command {
    CreateGroup(Group),
    DeleteGroup(u32),
    RouteStream { stream_id: u32, group_id: u32 },
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

struct Engine {
    core: pw::core::CoreRc,
    master_gain: Arc<Smoothed>,
    groups: HashMap<u32, GroupNode>,
    ports: HashMap<u32, PortInfo>,
    /// Name of the factory that creates Link objects, discovered from the
    /// registry the same way `create-delete-remote-objects.rs` does — the
    /// literal string is conventionally "link-factory" but isn't guaranteed.
    link_factory: Option<String>,
}

impl Engine {
    fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::CreateGroup(group) => self.create_group(&group),
            Command::DeleteGroup(id) => {
                self.groups.remove(&id);
            }
            Command::RouteStream { stream_id, group_id } => self.route_stream(stream_id, group_id),
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
        match build_group_node(&self.core, group, self.master_gain.clone()) {
            Ok(node) => {
                tracing::info!(id = group.id, name = %group.name, "group created");
                self.groups.insert(group.id, node);
            }
            Err(err) => tracing::error!(%err, id = group.id, "failed to create group"),
        }
    }

    fn route_stream(&self, stream_id: u32, group_id: u32) {
        let Some(node) = self.groups.get(&group_id) else {
            tracing::warn!(group_id, "route requested for unknown group");
            return;
        };
        let Some(factory) = self.link_factory.clone() else {
            tracing::warn!("no link factory discovered yet, can't route");
            return;
        };
        let input_node = node.capture.node_id();
        if input_node == u32::MAX {
            tracing::warn!(group_id, "group's capture node has no id yet, try again shortly");
            return;
        }

        // Linking node-to-node (no explicit ports) makes PipeWire try to
        // create fresh ports for the link instead of reusing our capture
        // stream's existing (format-negotiated, fixed) ones — observed
        // failing server-side with "no more port available". Pair up ports
        // by channel ourselves instead, the way `pw-link <src> <dst>` does.
        let mut outputs: Vec<_> = self
            .ports
            .iter()
            .filter(|(_, p)| p.node_id == stream_id && p.output)
            .collect();
        let mut inputs: Vec<_> = self
            .ports
            .iter()
            .filter(|(_, p)| p.node_id == input_node && !p.output)
            .collect();
        if outputs.is_empty() || inputs.is_empty() {
            tracing::warn!(stream_id, group_id, "stream or group has no ports yet, try again shortly");
            return;
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
                "link.input.node" => input_node.to_string(),
                // Keep the link alive on the server even though we drop our
                // local proxy to it immediately below.
                "object.linger" => "1",
            };
            match self.core.create_object::<pw::link::Link>(&factory, &props) {
                Ok(link) => std::mem::drop(link),
                Err(err) => tracing::error!(%err, out_id = **out_id, in_id, "failed to create link"),
            }
        }
        tracing::info!(stream_id, group_id, "routed stream into group");
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

    let capture_params = params.clone();
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
                    let mut x = f32::from_le_bytes(bytes) * gain;

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

                for sample in samples {
                    let _ = ring_tx.push(sample * mult);
                }
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

    let playback_listener = playback
        .add_local_listener_with_user_data(())
        .process(move |stream, _| {
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }
            let data = &mut datas[0];
            let stride = CHANNELS * std::mem::size_of::<f32>();
            let Some(slice) = data.data() else { return };
            let n_frames = slice.len() / stride;

            for frame in 0..n_frames {
                for ch in 0..CHANNELS {
                    let sample = ring_rx.pop().unwrap_or(0.0);
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
) -> Result<(), pw::Error> {
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;

    let engine = Rc::new(RefCell::new(Engine {
        core: core.clone(),
        master_gain: Arc::new(Smoothed::new(cfg.master_gain, SAMPLE_RATE, PARAM_RAMP_MS)),
        groups: HashMap::new(),
        ports: HashMap::new(),
        link_factory: None,
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
            engine_for_remove.borrow_mut().ports.remove(&id);
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
        engine.borrow_mut().ports.insert(global.id, PortInfo { node_id, output, channel });
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
    tracing::info!(id = global.id, name, "app stream");
    streams.lock().unwrap().insert(global.id, AppStream { id: global.id, name });
}
