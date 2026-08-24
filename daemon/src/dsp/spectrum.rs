//! Live frequency-analysis for the Home tab's spectrum graph and the MTX
//! Panel's "show me the input so I know what to EQ" overlay (both asked
//! for after the fact — not in PROJECT_SPEC.md's original stage list).
//! Analyzes the group's *input* (pre gain/EQ/dynamics), not its output —
//! the point is to see what's arriving before deciding how to shape it.
//!
//! Everything here is preallocated once at group-creation time and mutated
//! in place; `maybe_compute` never allocates, so it's safe to call from the
//! audio callback (Section 6's real-time rules) — same discipline as
//! `dsp::biquad`'s coefficient rebuilds.

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use std::f32::consts::PI;
use std::sync::Arc;

pub const N_BINS: usize = 24;
const FFT_SIZE: usize = 1024;
const FREQ_LO: f32 = 40.0;
const FREQ_HI: f32 = 18_000.0;
pub const FLOOR_DB: f32 = -60.0;

pub struct SpectrumAnalyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    ring: Vec<f32>,
    ring_pos: usize,
    scratch: Vec<Complex32>,
    countdown: u32,
    recompute_every: u32,
    sample_rate: f32,
}

impl SpectrumAnalyzer {
    /// `recompute_every`: only run the FFT once every N buffer callbacks —
    /// no UI polls fast enough to see every one, and it's one less thing
    /// competing for the single mainloop thread's time budget per group.
    pub fn new(sample_rate: f32, recompute_every: u32) -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        // Hann window: tapers the analyzed block to ~0 at both edges so the
        // FFT doesn't see a fake sharp edge (spectral leakage) where the
        // ring buffer's snapshot starts/ends.
        let window = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / (FFT_SIZE as f32 - 1.0)).cos())
            .collect();
        Self {
            fft,
            window,
            ring: vec![0.0; FFT_SIZE],
            ring_pos: 0,
            scratch: vec![Complex32::new(0.0, 0.0); FFT_SIZE],
            countdown: 0,
            recompute_every: recompute_every.max(1),
            sample_rate,
        }
    }

    /// Feed one raw input sample (call once per frame, before this group's
    /// own gain/EQ touch it).
    #[inline]
    pub fn push(&mut self, sample: f32) {
        self.ring[self.ring_pos] = sample;
        self.ring_pos = (self.ring_pos + 1) % FFT_SIZE;
    }

    /// Call once per buffer callback. `Some` only on the throttled cycles
    /// that actually recompute — `None` the rest of the time, so the
    /// caller doesn't need its own counter.
    pub fn maybe_compute(&mut self) -> Option<[f32; N_BINS]> {
        self.countdown += 1;
        if self.countdown < self.recompute_every {
            return None;
        }
        self.countdown = 0;

        for i in 0..FFT_SIZE {
            let s = self.ring[(self.ring_pos + i) % FFT_SIZE];
            self.scratch[i] = Complex32::new(s * self.window[i], 0.0);
        }
        self.fft.process(&mut self.scratch);

        let bin_hz = self.sample_rate / FFT_SIZE as f32;
        let log_lo = FREQ_LO.ln();
        let log_hi = FREQ_HI.ln();
        let mut bins = [FLOOR_DB; N_BINS];
        for (b, out) in bins.iter_mut().enumerate() {
            let f_lo = (log_lo + (b as f32 / N_BINS as f32) * (log_hi - log_lo)).exp();
            let f_hi = (log_lo + ((b + 1) as f32 / N_BINS as f32) * (log_hi - log_lo)).exp();
            let i_lo = ((f_lo / bin_hz) as usize).max(1);
            let i_hi = ((f_hi / bin_hz) as usize).min(FFT_SIZE / 2).max(i_lo + 1);
            let peak = self.scratch[i_lo..i_hi].iter().fold(0f32, |m, c| m.max(c.norm()));
            // Rough (not calibrated-to-dBFS) normalization — good enough
            // for a visual "where's the energy" display, not a measurement
            // tool.
            let db = 20.0 * (peak / (FFT_SIZE as f32 / 2.0) + 1e-9).log10();
            *out = db.clamp(FLOOR_DB, 0.0);
        }
        Some(bins)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_wave_peaks_in_the_right_bin() {
        let sample_rate = 48_000.0;
        let mut analyzer = SpectrumAnalyzer::new(sample_rate, 1);
        let freq = 1000.0; // should land solidly inside one of the mid bins
        for i in 0..FFT_SIZE {
            analyzer.push((2.0 * PI * freq * i as f32 / sample_rate).sin());
        }
        let bins = analyzer.maybe_compute().expect("recompute_every=1 always fires");

        let (loudest, _) =
            bins.iter().enumerate().max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap()).unwrap();
        // Bin `loudest` should correspond to a frequency range containing 1kHz.
        let log_lo = FREQ_LO.ln();
        let log_hi = FREQ_HI.ln();
        let f_lo = (log_lo + (loudest as f32 / N_BINS as f32) * (log_hi - log_lo)).exp();
        let f_hi = (log_lo + ((loudest + 1) as f32 / N_BINS as f32) * (log_hi - log_lo)).exp();
        assert!(f_lo <= freq && freq <= f_hi, "1kHz sine peaked in bin covering {f_lo}..{f_hi}Hz");
        assert!(bins[loudest] > FLOOR_DB, "peak bin should be well above the noise floor");
    }

    #[test]
    fn silence_stays_at_the_floor() {
        let mut analyzer = SpectrumAnalyzer::new(48_000.0, 1);
        for _ in 0..FFT_SIZE {
            analyzer.push(0.0);
        }
        let bins = analyzer.maybe_compute().unwrap();
        assert!(bins.iter().all(|&db| db <= FLOOR_DB + 1e-3));
    }
}
