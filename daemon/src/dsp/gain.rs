//! Smoothed gain (PROJECT_SPEC.md Section 6, build-order stage 3): the
//! reference implementation of the "no instant parameter jumps" rule. Every
//! other smoothed parameter (EQ coefficients, compressor threshold, ...)
//! should follow this same shape: an `AtomicU32`-backed target the control
//! thread writes, ramped toward on the audio thread one sample at a time.

use std::sync::atomic::{AtomicU32, Ordering};

pub struct SmoothedGain {
    current: f32,
    target: AtomicU32,
    /// Max change in gain per sample. Derived from sample rate so a ramp
    /// always takes ~`ramp_ms`, independent of buffer size.
    max_step: f32,
}

impl SmoothedGain {
    pub fn new(initial: f32, sample_rate: f32, ramp_ms: f32) -> Self {
        Self {
            current: initial,
            target: AtomicU32::new(initial.to_bits()),
            max_step: 1.0 / (ramp_ms * 0.001 * sample_rate),
        }
    }

    /// Control-thread side (e.g. a D-Bus `SetGain` call). Lock-free.
    pub fn set_target(&self, value: f32) {
        self.target.store(value.to_bits(), Ordering::Relaxed);
    }

    /// Audio-thread side: advance one sample, return the gain to apply.
    #[inline]
    pub fn next(&mut self) -> f32 {
        let target = f32::from_bits(self.target.load(Ordering::Relaxed));
        let diff = target - self.current;
        if diff.abs() <= self.max_step {
            self.current = target;
        } else {
            self.current += self.max_step.copysign(diff);
        }
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramps_instead_of_jumping() {
        let mut g = SmoothedGain::new(0.0, 48_000.0, 10.0); // 10ms ramp
        g.set_target(1.0);

        let first = g.next();
        assert!(first > 0.0 && first < 1.0, "must not jump instantly");

        // 10ms at 48kHz = 480 samples; give it slack and confirm convergence.
        for _ in 0..1000 {
            g.next();
        }
        assert!((g.next() - 1.0).abs() < 1e-6);
    }
}
