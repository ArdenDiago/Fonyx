//! Intelligent Audio Ducking (NewImprovenments/04-intelligent-audio-ducking.md):
//! automatically reduces selected groups when a priority group becomes
//! active, restoring them smoothly afterward.
//!
//! Detection: "RMS/peak envelope detection" (the spec's method 2 — no
//! hard-coded single method), reusing the trigger group's own live peak
//! level (`pw::GroupLive`, already published every audio buffer for the
//! Home tab's meters) rather than adding a second signal path. Method 1
//! (an explicit activity flag from a known source) and method 3 (a future
//! plugin detector) are left as extension points, not built — there's
//! exactly one real detector here, not a switchable enum with one live arm.
//!
//! `DuckingEngine::evaluate` is a pure function (no PipeWire access) called
//! from a ~20ms PipeWire mainloop timer (`pw.rs`) rather than per-sample —
//! attack/hold/release are computed at that tick granularity, which keeps
//! the state genuinely shared across every group's independent audio
//! callback (something a per-sample smoother inside one group's own
//! closure can't do, since ducking a *target* group depends on a
//! *different* group's trigger level) without needing cross-closure shared
//! mutable state on the real-time path. 20ms is coarse enough that a
//! reader should not expect sample-accurate envelope timing, but fine
//! enough that typical attack/hold/release values (tens to hundreds of ms)
//! don't audibly step.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuckingRule {
    pub id: u32,
    #[serde(default = "super_true")]
    pub enabled: bool,
    /// The group whose activity triggers ducking (e.g. a Discord/voice
    /// group).
    pub trigger_group: u32,
    pub target_groups: Vec<u32>,
    /// How much to reduce targets by, in dB (positive number = that many dB
    /// quieter).
    pub reduction_db: f32,
    pub attack_ms: f32,
    pub hold_ms: f32,
    pub release_ms: f32,
    /// Peak level (dBFS) above which the trigger counts as "active".
    pub threshold_db: f32,
}

fn super_true() -> bool {
    true
}

impl Default for DuckingRule {
    fn default() -> Self {
        Self { id: 0, enabled: true, trigger_group: 0, target_groups: Vec::new(), reduction_db: 12.0, attack_ms: 50.0, hold_ms: 200.0, release_ms: 500.0, threshold_db: -40.0 }
    }
}

/// Per-target running state: current applied linear gain and remaining
/// hold-open ticks, mirroring `dsp::dynamics::DynamicsProcessor`'s own
/// gate-hold shape but at tick (not sample) granularity.
#[derive(Default)]
struct TargetState {
    current_gain: f32,
    hold_remaining_ticks: u32,
}

#[derive(Default)]
pub struct DuckingEngine {
    state: HashMap<u32, TargetState>,
}

fn tick_coeff(time_ms: f32, tick_seconds: f32) -> f32 {
    let tau = (time_ms.max(0.01) * 0.001 / tick_seconds).max(1.0);
    1.0 - (-1.0 / tau).exp()
}

impl DuckingEngine {
    /// `trigger_peak_linear(group_id)` returns that group's current peak
    /// amplitude (0..1-ish), if it's live. Returns the linear gain multiplier
    /// every *target* group should currently apply — groups with no
    /// applicable rule aren't present in the map (callers should treat a
    /// missing entry as unity).
    pub fn evaluate(
        &mut self,
        rules: &[DuckingRule],
        trigger_peak_linear: impl Fn(u32) -> Option<f32>,
        tick_seconds: f32,
    ) -> HashMap<u32, f32> {
        // Strongest (lowest) requested reduction per target wins when more
        // than one rule targets the same group, computed before touching
        // any envelope state so simultaneous triggers (spec's own test
        // list) combine deterministically rather than by rule order.
        let mut wanted: HashMap<u32, (f32, &DuckingRule)> = HashMap::new();
        for rule in rules.iter().filter(|r| r.enabled) {
            let peak = trigger_peak_linear(rule.trigger_group).unwrap_or(0.0);
            let peak_db = 20.0 * peak.max(1e-9).log10();
            let active = peak_db > rule.threshold_db;
            let target_linear = if active { db_to_linear(-rule.reduction_db.abs()) } else { 1.0 };
            for &target_group in &rule.target_groups {
                let entry = wanted.entry(target_group).or_insert((1.0, rule));
                if target_linear < entry.0 {
                    *entry = (target_linear, rule);
                }
            }
        }

        let mut out = HashMap::new();
        for (&group_id, &(target_linear, rule)) in &wanted {
            let state = self.state.entry(group_id).or_insert_with(|| TargetState { current_gain: 1.0, hold_remaining_ticks: 0 });
            let is_ducking_deeper = target_linear < state.current_gain;
            if is_ducking_deeper {
                state.hold_remaining_ticks = (rule.hold_ms / (tick_seconds * 1000.0)).round() as u32;
                let coeff = tick_coeff(rule.attack_ms, tick_seconds);
                state.current_gain += (target_linear - state.current_gain) * coeff;
            } else if state.hold_remaining_ticks > 0 {
                state.hold_remaining_ticks -= 1;
            } else {
                let coeff = tick_coeff(rule.release_ms, tick_seconds);
                state.current_gain += (target_linear - state.current_gain) * coeff;
            }
            out.insert(group_id, state.current_gain);
        }
        // Any group that used to be ducked but no longer has a rule wanting
        // to duck it this tick should still release smoothly rather than
        // snapping back — keep evaluating its state (release toward unity)
        // until it's settled, then drop it to stop paying for it forever.
        let settled: Vec<u32> = self
            .state
            .iter()
            .filter(|(id, s)| !wanted.contains_key(id) && (s.current_gain - 1.0).abs() > 1e-4)
            .map(|(&id, _)| id)
            .collect();
        for group_id in settled {
            let default_release = DuckingRule::default().release_ms;
            let state = self.state.get_mut(&group_id).unwrap();
            let coeff = tick_coeff(default_release, tick_seconds);
            state.current_gain += (1.0 - state.current_gain) * coeff;
            out.insert(group_id, state.current_gain);
        }
        out
    }
}

fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule() -> DuckingRule {
        DuckingRule { id: 1, trigger_group: 1, target_groups: vec![2], reduction_db: 12.0, attack_ms: 20.0, hold_ms: 100.0, release_ms: 200.0, threshold_db: -40.0, enabled: true }
    }

    #[test]
    fn ducks_target_when_trigger_is_loud() {
        let mut engine = DuckingEngine::default();
        let rules = vec![rule()];
        // Loud trigger (0dBFS), run enough ticks for the attack to settle.
        let mut last = 1.0;
        for _ in 0..50 {
            let out = engine.evaluate(&rules, |g| if g == 1 { Some(1.0) } else { None }, 0.02);
            last = *out.get(&2).unwrap();
        }
        let expected = db_to_linear(-12.0);
        assert!((last - expected).abs() < 0.01, "should settle near -12dB, got {last} vs {expected}");
    }

    #[test]
    fn does_not_duck_when_trigger_is_quiet() {
        let mut engine = DuckingEngine::default();
        let rules = vec![rule()];
        let out = engine.evaluate(&rules, |g| if g == 1 { Some(0.0) } else { None }, 0.02);
        assert_eq!(*out.get(&2).unwrap_or(&1.0), 1.0);
    }

    #[test]
    fn holds_before_releasing() {
        let mut engine = DuckingEngine::default();
        let rules = vec![rule()];
        // Duck it down first.
        for _ in 0..50 {
            engine.evaluate(&rules, |_| Some(1.0), 0.02);
        }
        // Trigger goes quiet — should still be ducked immediately after
        // (hold_ms = 100ms = 5 ticks at 20ms).
        let out = engine.evaluate(&rules, |_| Some(0.0), 0.02);
        let still_ducked = *out.get(&2).unwrap();
        assert!(still_ducked < 0.9, "should still be held down right after trigger stops, got {still_ducked}");
    }

    #[test]
    fn combines_simultaneous_triggers_by_strongest_reduction() {
        let mut engine = DuckingEngine::default();
        let mut r2 = rule();
        r2.id = 2;
        r2.trigger_group = 3;
        r2.reduction_db = 20.0; // stronger
        let rules = vec![rule(), r2];
        let mut last = 1.0;
        for _ in 0..100 {
            let out = engine.evaluate(&rules, |g| (g == 1 || g == 3).then_some(1.0), 0.02);
            last = *out.get(&2).unwrap();
        }
        let expected = db_to_linear(-20.0);
        assert!((last - expected).abs() < 0.01, "strongest (lowest) requested gain should win, got {last} vs {expected}");
    }
}
