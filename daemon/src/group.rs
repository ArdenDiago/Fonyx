//! Group data model: a user-defined bus that app streams route into, with
//! its own DSP chain (PROJECT_SPEC.md Section 4, 6). This is the plain
//! serializable state; the live PipeWire node + per-sample processing that
//! stage 2+ attaches to a group lives in `pw.rs` / `dsp/`.

use serde::{Deserialize, Serialize};

use crate::dsp::{
    biquad::EqBand,
    dynamics::{CompressorParams, GateParams},
};

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
    pub compressor: CompressorParams,
    #[serde(default)]
    pub gate: GateParams,
    /// PipeWire node IDs of app streams currently routed into this group.
    #[serde(default)]
    pub routed_streams: Vec<u32>,
}

impl Group {
    pub fn new(id: u32, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            gain: 1.0,
            eq_bands: Vec::new(),
            compressor: CompressorParams::default(),
            gate: GateParams::default(),
            routed_streams: Vec::new(),
        }
    }
}
