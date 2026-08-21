//! EQ biquads (PROJECT_SPEC.md Section 6/7.3): RBJ cookbook filters. `EqBand`
//! and `FilterSlope` are the persisted/D-Bus-facing data; `Biquad` is the
//! per-sample processor a group's DSP chain holds one of per band/filter.
//!
//! Coefficient updates (`set_peaking`/`set_highpass`/`set_lowpass`) mutate
//! only the coefficients, never the `x1/x2/y1/y2` filter history — recreating
//! a `Biquad` from scratch on every parameter tweak would reset that history
//! and click. The chain recomputes coefficients once per buffer from
//! `Smoothed`-ramped parameter values, so changes still arrive gradually.

use serde::{Deserialize, Serialize};
use std::f32::consts::PI;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EqBand {
    pub freq_hz: f32,
    pub gain_db: f32,
    pub q: f32,
    pub enabled: bool,
}

impl Default for EqBand {
    fn default() -> Self {
        Self { freq_hz: 1000.0, gain_db: 0.0, q: 1.0, enabled: false }
    }
}

/// A High-pass or Low-pass filter (PROJECT_SPEC.md Section 7.3's HPF/LPF
/// section) — same shape as `EqBand` minus gain, since a cutoff filter has
/// none.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FilterSlope {
    pub freq_hz: f32,
    pub q: f32,
    pub enabled: bool,
}

impl Default for FilterSlope {
    fn default() -> Self {
        Self { freq_hz: 80.0, q: std::f32::consts::FRAC_1_SQRT_2, enabled: false }
    }
}

/// Direct Form I biquad.
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
    fn set_coeffs(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    /// RBJ cookbook peaking EQ coefficients.
    pub fn set_peaking(&mut self, band: &EqBand, sample_rate: f32) {
        let a = 10f32.powf(band.gain_db / 40.0);
        let w0 = 2.0 * PI * band.freq_hz / sample_rate;
        let alpha = w0.sin() / (2.0 * band.q);
        let cos_w0 = w0.cos();

        self.set_coeffs(
            1.0 + alpha * a,
            -2.0 * cos_w0,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos_w0,
            1.0 - alpha / a,
        );
    }

    /// RBJ cookbook high-pass coefficients.
    pub fn set_highpass(&mut self, freq_hz: f32, q: f32, sample_rate: f32) {
        let w0 = 2.0 * PI * freq_hz / sample_rate;
        let alpha = w0.sin() / (2.0 * q);
        let cos_w0 = w0.cos();

        self.set_coeffs(
            (1.0 + cos_w0) / 2.0,
            -(1.0 + cos_w0),
            (1.0 + cos_w0) / 2.0,
            1.0 + alpha,
            -2.0 * cos_w0,
            1.0 - alpha,
        );
    }

    /// RBJ cookbook low-pass coefficients.
    pub fn set_lowpass(&mut self, freq_hz: f32, q: f32, sample_rate: f32) {
        let w0 = 2.0 * PI * freq_hz / sample_rate;
        let alpha = w0.sin() / (2.0 * q);
        let cos_w0 = w0.cos();

        self.set_coeffs(
            (1.0 - cos_w0) / 2.0,
            1.0 - cos_w0,
            (1.0 - cos_w0) / 2.0,
            1.0 + alpha,
            -2.0 * cos_w0,
            1.0 - alpha,
        );
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
        let mut f = Biquad::default();
        f.set_peaking(&band, 48_000.0);
        for _ in 0..10 {
            let out = f.process(1.0);
            assert!((out - 1.0).abs() < 1e-3);
        }
    }

    #[test]
    fn recomputing_coeffs_preserves_history() {
        let band = EqBand { freq_hz: 1000.0, gain_db: 6.0, q: 1.0, enabled: true };
        let mut f = Biquad::default();
        f.set_peaking(&band, 48_000.0);
        f.process(1.0);
        f.process(-1.0);
        let history_before = (f.x1, f.x2, f.y1, f.y2);

        // Recompute coefficients (e.g. a smoothed freq tick) — history must
        // survive, unlike constructing a fresh Biquad would.
        let band2 = EqBand { freq_hz: 1005.0, ..band };
        f.set_peaking(&band2, 48_000.0);
        assert_eq!((f.x1, f.x2, f.y1, f.y2), history_before);
    }

    #[test]
    fn lowpass_attenuates_high_frequency() {
        let mut f = Biquad::default();
        f.set_lowpass(200.0, std::f32::consts::FRAC_1_SQRT_2, 48_000.0);
        // Settle, then measure response to a near-Nyquist alternating signal.
        for _ in 0..100 {
            f.process(0.0);
        }
        let mut max_out: f32 = 0.0;
        for i in 0..200 {
            let x = if i % 2 == 0 { 1.0 } else { -1.0 };
            max_out = max_out.max(f.process(x).abs());
        }
        assert!(max_out < 0.5, "lowpass should attenuate near-Nyquist content, got {max_out}");
    }
}
