//! Group data model: a user-defined bus that app streams route into, with
//! its own DSP chain (PROJECT_SPEC.md Section 4, 6). This is the plain
//! serializable state; the live PipeWire node + per-sample processing that
//! stage 2+ attaches to a group lives in `pw.rs` / `dsp/`.

use serde::{Deserialize, Serialize};

use crate::dsp::{
    biquad::{EqBand, FilterSlope},
    dynamics::{CompressorParams, GateParams},
};

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
        }
    }
}
