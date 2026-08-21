//! Smoothed parameter (PROJECT_SPEC.md Section 6, build-order stage 3): the
//! reference implementation of the "no instant parameter jumps" rule. Every
//! UI-driven parameter (gain, EQ band freq/gain/Q, compressor/gate knobs)
//! uses one of these: an `AtomicU32`-backed target the control thread
//! writes, ramped toward on the audio thread one sample at a time.
//!
//! One-pole exponential ramp rather than linear: it's scale-invariant (the
//! same `ramp_ms` setting works whether the value being ramped is a 0..2
//! gain or a 20..20000 Hz frequency), it can't overshoot, and it's what the
//! spec explicitly allows ("linear or exponential interpolation").
//!
//! Both `current` and `target` are atomics (not `&mut self` state) so a
//! `Smoothed` can live behind a plain `Arc`, shared between the control
//! thread (writes `target`) and the audio thread (advances `current`) with
//! no lock — even though only the audio thread ever actually calls `next`.

use std::sync::atomic::{AtomicU32, Ordering};

pub struct Smoothed {
    current: AtomicU32,
    target: AtomicU32,
    /// Per-sample decay coefficient: current += (target - current) * coeff.
    coeff: f32,
}

impl Smoothed {
    /// `ramp_ms` is the time to settle within ~1% of a new target.
    pub fn new(initial: f32, sample_rate: f32, ramp_ms: f32) -> Self {
        let time_constant_samples = (ramp_ms * 0.001 * sample_rate) / 5.0;
        Self {
            current: AtomicU32::new(initial.to_bits()),
            target: AtomicU32::new(initial.to_bits()),
            coeff: 1.0 / time_constant_samples.max(1.0),
        }
    }

    /// Control-thread side (e.g. a D-Bus `SetGain` call). Lock-free.
    pub fn set_target(&self, value: f32) {
        self.target.store(value.to_bits(), Ordering::Relaxed);
    }

    pub fn target(&self) -> f32 {
        f32::from_bits(self.target.load(Ordering::Relaxed))
    }

    pub fn current(&self) -> f32 {
        f32::from_bits(self.current.load(Ordering::Relaxed))
    }

    /// Audio-thread side: advance one sample, return the value to apply.
    /// Only ever called from the single stream-process thread that owns
    /// this parameter — no concurrent callers, so the load/store pair below
    /// can't race even though it isn't a single atomic RMW.
    #[inline]
    pub fn next(&self) -> f32 {
        let target = self.target();
        let current = self.current();
        let diff = target - current;
        // Snap once negligible (relative to target's own scale — an
        // absolute epsilon is meaningless once target is e.g. 8000.0, where
        // f32 rounding stalls the ramp well above a fixed 1e-7 and it would
        // never snap at all): avoids denormal-float slowdowns forever
        // chasing an asymptote it never quite reaches.
        let epsilon = (target.abs() * 1e-4).max(1e-6);
        let next = if diff.abs() < epsilon { target } else { current + diff * self.coeff };
        self.current.store(next.to_bits(), Ordering::Relaxed);
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramps_instead_of_jumping() {
        let g = Smoothed::new(0.0, 48_000.0, 10.0); // 10ms ramp
        g.set_target(1.0);

        let first = g.next();
        assert!(first > 0.0 && first < 1.0, "must not jump instantly");

        for _ in 0..10_000 {
            g.next();
        }
        assert_eq!(g.next(), 1.0);
    }

    #[test]
    fn works_for_non_unit_ranges() {
        // e.g. an EQ band frequency ramping from 100Hz to 8000Hz.
        let f = Smoothed::new(100.0, 48_000.0, 10.0);
        f.set_target(8000.0);
        for _ in 0..10_000 {
            f.next();
        }
        assert_eq!(f.next(), 8000.0);
    }
}
