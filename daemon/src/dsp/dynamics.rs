//! Compressor + gate (PROJECT_SPEC.md Section 6/7.3, build-order stage 6).
//! Feed-forward gain-computer topology: compute the target gain reduction
//! instantaneously from signal level, then smooth the *gain* itself toward
//! that target using attack/release one-pole coefficients — simpler than
//! smoothing the level detector, and it's what keeps both the compressor
//! and the gate glitch-free without a separate crossfade mechanism.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CompressorParams {
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub makeup_gain_db: f32,
    #[serde(default)]
    pub enabled: bool,
}

impl Default for CompressorParams {
    fn default() -> Self {
        Self {
            threshold_db: -18.0,
            ratio: 2.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            makeup_gain_db: 0.0,
            enabled: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GateParams {
    pub threshold_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub hold_ms: f32,
    #[serde(default)]
    pub enabled: bool,
}

impl Default for GateParams {
    fn default() -> Self {
        Self {
            threshold_db: -50.0,
            attack_ms: 2.0,
            release_ms: 150.0,
            hold_ms: 50.0,
            enabled: false,
        }
    }
}

const GATE_FLOOR_DB: f32 = -80.0;

pub struct DynamicsProcessor {
    comp_gain_db: f32,
    gate_gain_db: f32,
    gate_hold_remaining: u32,
}

impl DynamicsProcessor {
    pub fn new() -> Self {
        Self { comp_gain_db: 0.0, gate_gain_db: 0.0, gate_hold_remaining: 0 }
    }

    /// Current compressor gain reduction in dB (<=0). For the Home tab's
    /// gain-reduction graph (Section 7.1) once that's wired up.
    pub fn gain_reduction_db(&self) -> f32 {
        self.comp_gain_db.min(0.0)
    }

    /// Current gate attenuation in dB (<=0; 0 when open). Same shape as
    /// `gain_reduction_db` but for the Gate sub-tab's own meter.
    pub fn gate_reduction_db(&self) -> f32 {
        self.gate_gain_db.min(0.0)
    }

    #[inline]
    pub fn process(
        &mut self,
        x: f32,
        comp: &CompressorParams,
        gate: &GateParams,
        sample_rate: f32,
    ) -> f32 {
        let level_db = 20.0 * (x.abs() + 1e-9).log10();

        let comp_target = if !comp.enabled {
            comp.makeup_gain_db
        } else if level_db > comp.threshold_db {
            let over = level_db - comp.threshold_db;
            comp.makeup_gain_db - over * (1.0 - 1.0 / comp.ratio.max(1.0))
        } else {
            comp.makeup_gain_db
        };
        let comp_attack = one_pole_coeff(comp.attack_ms, sample_rate);
        let comp_release = one_pole_coeff(comp.release_ms, sample_rate);
        self.comp_gain_db = smooth_towards(self.comp_gain_db, comp_target, comp_attack, comp_release);

        let gate_target = if !gate.enabled || level_db > gate.threshold_db {
            self.gate_hold_remaining = (gate.hold_ms * 0.001 * sample_rate) as u32;
            0.0
        } else if self.gate_hold_remaining > 0 {
            self.gate_hold_remaining -= 1;
            0.0
        } else {
            GATE_FLOOR_DB
        };
        let gate_attack = one_pole_coeff(gate.attack_ms, sample_rate);
        let gate_release = one_pole_coeff(gate.release_ms, sample_rate);
        // Gate opening (target rises toward 0) is the fast/attack path;
        // closing (target falls toward the floor) is release — the reverse
        // of the compressor's falling-is-attack convention.
        self.gate_gain_db = smooth_towards(self.gate_gain_db, gate_target, gate_release, gate_attack);

        x * db_to_linear(self.comp_gain_db + self.gate_gain_db)
    }
}

/// Always-on output safety net, not a user-configurable effect (no
/// `enabled` flag, no params exposed anywhere) — a fixed-ceiling limiter
/// applied after everything else in the chain (EQ, compressor makeup
/// gain, plugins), so no combination of user settings can make a group's
/// output audibly overshoot 0dBFS. A hard `clamp(-1.0, 1.0)` alone stops
/// the sample values from actually overflowing, but still *sounds* like
/// hard clipping every time it engages; this instead pulls gain down
/// smoothly just ahead of the ceiling and eases it back once the peak
/// passes, the same feed-forward gain-computer shape `DynamicsProcessor`
/// already uses, just tuned for "prevent, don't just clip" instead of
/// creative compression. Fast enough attack to catch most real transients
/// before they clip; the hard clamp stays in place as the absolute
/// backstop for the sliver a zero-lookahead limiter can't fully catch on
/// a truly instantaneous step.
pub struct Limiter {
    gain_db: f32,
}

impl Limiter {
    pub fn new() -> Self {
        Self { gain_db: 0.0 }
    }

    /// Current limiter gain reduction in dB (<=0). Exposed the same shape
    /// as `DynamicsProcessor`'s meters in case a future UI wants to show
    /// "the safety limiter is working" — not surfaced anywhere yet.
    pub fn gain_reduction_db(&self) -> f32 {
        self.gain_db.min(0.0)
    }

    /// `peak` should be the stereo-linked peak (max across channels, like
    /// `DynamicsProcessor::process`'s caller already computes) — one gain
    /// value applied to both channels, never an independent per-channel
    /// one, so a loud transient can't shift the stereo image.
    #[inline]
    pub fn process(&mut self, peak: f32, ceiling_db: f32, attack_ms: f32, release_ms: f32, sample_rate: f32) -> f32 {
        let level_db = 20.0 * (peak.abs() + 1e-9).log10();
        let target = (ceiling_db - level_db).min(0.0);
        let attack = one_pole_coeff(attack_ms, sample_rate);
        let release = one_pole_coeff(release_ms, sample_rate);
        self.gain_db = smooth_towards(self.gain_db, target, attack, release);
        db_to_linear(self.gain_db)
    }
}

fn one_pole_coeff(time_ms: f32, sample_rate: f32) -> f32 {
    let tau_samples = (time_ms.max(0.01) * 0.001 * sample_rate).max(1.0);
    1.0 - (-1.0 / tau_samples).exp()
}

/// `falling_coeff` applies when the target is below current (gain
/// decreasing), `rising_coeff` when it's above (gain increasing).
fn smooth_towards(current: f32, target: f32, falling_coeff: f32, rising_coeff: f32) -> f32 {
    let coeff = if target < current { falling_coeff } else { rising_coeff };
    current + (target - current) * coeff
}

fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressor_reduces_loud_signal() {
        let mut dsp = DynamicsProcessor::new();
        let comp = CompressorParams { threshold_db: -18.0, ratio: 4.0, attack_ms: 1.0, release_ms: 50.0, makeup_gain_db: 0.0, enabled: true };
        let gate = GateParams::default(); // disabled

        let mut last = 0.0;
        for _ in 0..10_000 {
            last = dsp.process(0.5, &comp, &gate, 48_000.0);
        }
        assert!(last < 0.5, "loud steady signal should be gain-reduced, got {last}");
        assert!(dsp.gain_reduction_db() < 0.0);
    }

    #[test]
    fn gate_silences_quiet_signal_after_hold() {
        let mut dsp = DynamicsProcessor::new();
        let comp = CompressorParams::default(); // disabled
        let gate = GateParams { threshold_db: -30.0, attack_ms: 1.0, release_ms: 5.0, hold_ms: 5.0, enabled: true };

        let mut last = 1.0;
        for _ in 0..10_000 {
            last = dsp.process(0.001, &comp, &gate, 48_000.0); // well below -30dB
        }
        assert!(last.abs() < 1e-4, "quiet signal should be gated to silence, got {last}");
    }

    #[test]
    fn limiter_holds_a_hot_steady_signal_at_the_ceiling() {
        let mut lim = Limiter::new();
        let ceiling_db = -0.3;
        let raw_peak = 2.0; // +6dBFS, held steady — exactly the "EQ boost
        // + compressor makeup gain" overshoot this exists to catch.
        let mut gain = 1.0;
        for _ in 0..10_000 {
            gain = lim.process(raw_peak, ceiling_db, 1.0, 50.0, 48_000.0);
        }
        let result_db = 20.0 * (raw_peak * gain).log10();
        assert!((result_db - ceiling_db).abs() < 0.05, "steady-state output should settle at the ceiling, got {result_db}dB");
        assert!(lim.gain_reduction_db() < 0.0);
    }

    #[test]
    fn limiter_leaves_a_quiet_signal_untouched() {
        let mut lim = Limiter::new();
        let mut last = 1.0;
        for _ in 0..1_000 {
            last = lim.process(0.1, -0.3, 1.0, 50.0, 48_000.0);
        }
        assert_eq!(last, 1.0, "a peak well under the ceiling should get no gain reduction at all");
    }
}
