//! Parametric EQ band (PROJECT_SPEC.md Section 6/7.3, build-order stage 5):
//! RBJ cookbook biquad. `EqBand` is the persisted/D-Bus-facing data; `Biquad`
//! is the per-sample processor a group's DSP chain will hold one of per band.
//! Cascading bands into a chain and hooking this up to the audio callback is
//! stage 5 work, not done yet — this gives the math a single place to live
//! and be unit-tested ahead of that.

use serde::{Deserialize, Serialize};
use std::f32::consts::PI;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EqBand {
    pub freq_hz: f32,
    pub gain_db: f32,
    pub q: f32,
    pub enabled: bool,
}

/// Direct Form I biquad. Coefficients are set directly (not smoothed here —
/// smoothing coefficient *transitions* click-free requires crossfading two
/// filter states, which stage 5 will build on top of this using the same
/// `SmoothedGain`-style pattern per parameter).
#[derive(Debug, Clone, Copy, Default)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    /// RBJ cookbook peaking EQ coefficients.
    pub fn peaking(band: &EqBand, sample_rate: f32) -> Self {
        let a = 10f32.powf(band.gain_db / 40.0);
        let w0 = 2.0 * PI * band.freq_hz / sample_rate;
        let alpha = w0.sin() / (2.0 * band.q);

        let cos_w0 = w0.cos();
        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            ..Default::default()
        }
    }

    #[inline]
    pub fn process(&mut self, x0: f32) -> f32 {
        let y0 = self.b0 * x0 + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x0;
        self.y2 = self.y1;
        self.y1 = y0;
        y0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_gain_band_is_near_identity() {
        let band = EqBand { freq_hz: 1000.0, gain_db: 0.0, q: 1.0, enabled: true };
        let mut f = Biquad::peaking(&band, 48_000.0);
        for _ in 0..10 {
            let out = f.process(1.0);
            assert!((out - 1.0).abs() < 1e-3);
        }
    }
}
