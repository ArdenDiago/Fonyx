//! Per-output-route delay line
//! (NewImprovenments/MTX_MULTI_OUTPUT_DELAY_COMPENSATION.md §1/§11/§21): adds
//! a small, smoothly-adjustable amount of delay to one route's playback so
//! multiple outputs fed from the same group can be made to arrive closer
//! together.
//!
//! Fixed-capacity ring buffer, pre-allocated once at route creation (no
//! allocation in the audio callback — Section 21's real-time requirement).
//! Delay changes are slewed at most one sample of drift per output sample
//! (Section 11: "avoid... being applied instantly to a live signal") rather
//! than snapping, and reads use linear interpolation between the two nearest
//! taps so a still-slewing (fractional-sample) delay doesn't stair-step.

pub struct DelayLine {
    buffer: Vec<f32>,
    write_pos: usize,
    current_delay_samples: f32,
    target_delay_samples: f32,
    max_delay_samples: f32,
}

impl DelayLine {
    /// `max_delay_samples` bounds both the buffer size and the largest delay
    /// that can ever be requested (MTX_MULTI_OUTPUT_DELAY_COMPENSATION.md
    /// §12's configurable maximum compensation) — a `set_delay_ms` beyond
    /// this is clamped, never silently grown.
    pub fn new(max_delay_samples: usize) -> Self {
        // +2 guard samples: linear interpolation reads one tap past the
        // floor index, and the max-delay clamp can land exactly on the last
        // valid index.
        Self {
            buffer: vec![0.0; max_delay_samples.max(1) + 2],
            write_pos: 0,
            current_delay_samples: 0.0,
            target_delay_samples: 0.0,
            max_delay_samples: max_delay_samples as f32,
        }
    }

    pub fn set_delay_ms(&mut self, ms: f32, sample_rate: f32) {
        let samples = (ms.max(0.0) * 0.001 * sample_rate).min(self.max_delay_samples);
        self.target_delay_samples = samples;
    }

    /// Exposed for the Outputs UI's "Compensation" readout and for tests —
    /// the *actual* currently-applied delay, which lags `target` while
    /// slewing. Not yet read by any D-Bus method (no route is live-
    /// introspectable this pass beyond its configured target), so the
    /// non-test build doesn't call it yet.
    #[allow(dead_code)]
    pub fn current_delay_ms(&self, sample_rate: f32) -> f32 {
        self.current_delay_samples / sample_rate * 1000.0
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        self.buffer[self.write_pos] = input;

        let diff = self.target_delay_samples - self.current_delay_samples;
        const MAX_STEP_PER_SAMPLE: f32 = 1.0;
        self.current_delay_samples = if diff.abs() <= MAX_STEP_PER_SAMPLE {
            self.target_delay_samples
        } else {
            self.current_delay_samples + diff.signum() * MAX_STEP_PER_SAMPLE
        };

        let cap = self.buffer.len();
        let read_pos_f = self.write_pos as f32 - self.current_delay_samples;
        let idx0 = (read_pos_f.floor() as i64).rem_euclid(cap as i64) as usize;
        let idx1 = (idx0 + 1) % cap;
        let frac = read_pos_f - read_pos_f.floor();
        let out = self.buffer[idx0] * (1.0 - frac) + self.buffer[idx1] * frac;

        self.write_pos = (self.write_pos + 1) % cap;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_at_zero_delay() {
        let mut d = DelayLine::new(100);
        assert_eq!(d.process(0.5), 0.5);
        assert_eq!(d.process(-0.25), -0.25);
    }

    #[test]
    fn delays_an_impulse_by_the_settled_amount() {
        let mut d = DelayLine::new(100);
        d.set_delay_ms(3.0 / 48_000.0 * 1000.0, 48_000.0); // 3 samples
        for _ in 0..20 {
            d.process(0.0);
        }
        assert_eq!(d.current_delay_ms(48_000.0), 3.0 / 48_000.0 * 1000.0);

        let mut out = Vec::new();
        out.push(d.process(1.0));
        for _ in 0..9 {
            out.push(d.process(0.0));
        }
        let peak = out.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
        assert_eq!(peak, 3, "impulse should reappear 3 samples later, got peak at {peak}: {out:?}");
    }

    #[test]
    fn slews_gradually_instead_of_jumping() {
        let mut d = DelayLine::new(1000);
        d.set_delay_ms(100.0 / 48_000.0 * 1000.0, 48_000.0); // 100-sample target
        d.process(0.0);
        assert_eq!(d.current_delay_samples, 1.0, "should move at most one sample toward target per call");
        for _ in 0..200 {
            d.process(0.0);
        }
        // Not exact equality: `set_delay_ms` round-trips through a ms value
        // itself derived from a sample count in this test, and that
        // ms<->samples conversion isn't bit-exact in f32 — a sub-sample
        // rounding difference here is inaudible and expected.
        assert!(
            (d.current_delay_samples - 100.0).abs() < 0.01,
            "should reach ~the target once given enough samples, got {}",
            d.current_delay_samples
        );
    }

    #[test]
    fn clamps_to_max_delay() {
        let mut d = DelayLine::new(10);
        d.set_delay_ms(1000.0, 48_000.0); // way more than 10 samples' worth
        assert_eq!(d.target_delay_samples, 10.0);
    }
}
