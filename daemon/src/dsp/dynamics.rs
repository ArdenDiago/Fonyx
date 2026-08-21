//! Compressor/gate params (PROJECT_SPEC.md Section 7.3, build-order stage
//! 6). Data only for now — the envelope follower + gain computer that reads
//! these and processes audio is stage 6 work, done after EQ.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CompressorParams {
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub makeup_gain_db: f32,
}

impl Default for CompressorParams {
    fn default() -> Self {
        Self {
            threshold_db: -18.0,
            ratio: 2.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            makeup_gain_db: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GateParams {
    pub threshold_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub hold_ms: f32,
}

impl Default for GateParams {
    fn default() -> Self {
        Self {
            threshold_db: -50.0,
            attack_ms: 2.0,
            release_ms: 150.0,
            hold_ms: 50.0,
        }
    }
}
