//! Group data model: a user-defined bus that app streams route into, with
//! its own DSP chain (PROJECT_SPEC.md Section 4, 6). This is the plain
//! serializable state; the live PipeWire node + per-sample processing that
//! stage 2+ attaches to a group lives in `pw.rs` / `dsp/`.

use serde::{Deserialize, Serialize};

use crate::dsp::{
    biquad::{EqBand, FilterSlope},
    dynamics::{CompressorParams, GateParams},
};
use crate::identity::OutputId;
use crate::output::{RouteMode, SyncMode};
use crate::plugins::PluginInstance;

/// Staggered center frequencies for the default 6 peaking bands, matching
/// PipeWire's own `sink-eq6.conf` reference EQ layout.
const DEFAULT_BAND_FREQS: [f32; 6] = [100.0, 300.0, 1_000.0, 3_000.0, 8_000.0, 12_000.0];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: u32,
    pub name: String,
    /// Linear gain multiplier (1.0 = unity). Applied through the smoothed
    /// gain stage described in Section 6, not directly.
    pub gain: f32,
    #[serde(default)]
    pub eq_bands: Vec<EqBand>,
    #[serde(default)]
    pub highpass: FilterSlope,
    #[serde(default)]
    pub lowpass: FilterSlope,
    #[serde(default)]
    pub compressor: CompressorParams,
    #[serde(default)]
    pub gate: GateParams,
    /// PipeWire node IDs of app streams currently routed into this group.
    #[serde(default)]
    pub routed_streams: Vec<u32>,
    /// The always-on catch-all group (Section: default live-audio bus) —
    /// exactly one exists (`Config::ensure_default_group`), can't be
    /// deleted (`dbus.rs::delete_group` refuses), and every newly-detected
    /// app stream auto-routes into it unless the user moves it elsewhere
    /// (`pw.rs::on_global`).
    #[serde(default)]
    pub protected: bool,
    /// The permanent "Cue" group (DJ-style solo/cue bus): exactly one
    /// exists (`Config::ensure_cue_group`), can't be deleted, its routing
    /// is owned by `Config::cue_outputs` (not user-editable), and it is
    /// never summed into Master — so cued audio only reaches cue outputs.
    #[serde(default)]
    pub cue: bool,
    /// This group's outputs (MTX_AUDIO_GROUPS_MULTI_OUTPUT.md §7) — empty
    /// means "no explicit routing configured", which keeps this group on
    /// its original behavior: one playback stream, auto-connected to
    /// whatever PipeWire's own session manager currently calls the default
    /// sink. Existing groups from before this feature deserialize to an
    /// empty list here and see zero behavior change.
    #[serde(default)]
    pub output_routes: Vec<GroupOutputRoute>,
    /// Plugin chain slots (NewImprovenments/10-plugin-dsp-architecture.md),
    /// applied in order after gain/EQ/HPF-LPF/compressor/gate, before the
    /// signal reaches its output route(s). Empty by default — zero behavior
    /// change for a group nobody has added a plugin to.
    #[serde(default)]
    pub plugins: Vec<PluginInstance>,
}

/// One group's route to one output device (MTX_AUDIO_GROUPS_MULTI_OUTPUT.md
/// §7): its own gain/enable/sync state, independent of every other route
/// this group has and of the group's own master `gain`. Keyed by the
/// output's stable `OutputId`, never a transient PipeWire node id, so the
/// route survives the device disconnecting and reconnecting (§13).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupOutputRoute {
    pub output_id: OutputId,
    /// Linear gain multiplier for this route only. Effective gain reaching
    /// the device is `group.gain * route.gain` (§8).
    pub gain: f32,
    pub enabled: bool,
    #[serde(default)]
    pub mode: RouteMode,
    #[serde(default)]
    pub sync_mode: SyncMode,
    /// Manual compensation in ms (§4's Manual mode) — read only when
    /// `sync_mode == Manual`.
    #[serde(default)]
    pub manual_delay_ms: f32,
    /// Whether this route counts toward automatic-sync reference/
    /// compensation calculations (§13, "excluding an output").
    #[serde(default = "default_true")]
    pub sync_included: bool,
}

fn default_true() -> bool {
    true
}

impl GroupOutputRoute {
    pub fn new(output_id: OutputId, mode: RouteMode) -> Self {
        Self {
            output_id,
            gain: 1.0,
            enabled: true,
            mode,
            sync_mode: SyncMode::Off,
            manual_delay_ms: 0.0,
            sync_included: true,
        }
    }
}

impl Group {
    pub fn new(id: u32, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            gain: 1.0,
            // Enabled from the start — no separate "add a band" step before
            // a new group's EQ does anything; a user drags what's already
            // there instead of hunting for an add button first.
            eq_bands: DEFAULT_BAND_FREQS
                .into_iter()
                .map(|freq_hz| EqBand { freq_hz, enabled: true, ..Default::default() })
                .collect(),
            highpass: FilterSlope::default(),
            lowpass: FilterSlope::default(),
            compressor: CompressorParams::default(),
            gate: GateParams::default(),
            routed_streams: Vec::new(),
            protected: false,
            cue: false,
            output_routes: Vec::new(),
            plugins: Vec::new(),
        }
    }
}
