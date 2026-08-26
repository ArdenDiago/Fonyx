//! PipeWire Diagnostics / MTX Doctor
//! (NewImprovenments/08-pipewire-diagnostics-doctor.md): turns already-
//! tracked daemon state into a health report instead of adding a second
//! monitoring subsystem — every input here is something `dbus.rs` already
//! has on hand (`Config`, the live-group map, `Metrics`, the outputs map).
//!
//! Scope decision: node/port/link *counts* aren't surfaced (that state
//! lives only on the PipeWire engine thread, inside `Engine`, and isn't
//! mirrored anywhere the D-Bus thread can read it without adding yet
//! another shared field) — the checks below use what's already shared
//! (`live`, `outputs`, `Metrics`) instead, which covers the spec's own
//! checklist items that matter most for "what broke and can MTX fix it":
//! groups missing their PipeWire presence, buffer/underrun health, and
//! output availability.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Ok,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Ok => "ok",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

pub struct HealthCheck {
    pub name: String,
    pub severity: Severity,
    pub message: String,
}

#[derive(Default)]
pub struct HealthInputs {
    pub configured_group_ids: Vec<u32>,
    pub live_group_ids: Vec<u32>,
    pub buffer_frames: u32,
    /// Lifetime total (never resets) — shown for context, not used for
    /// severity. See `underrun_delta`'s doc comment for why.
    pub underrun_count: u64,
    /// Underruns-per-second *since the previous health check* (computed by
    /// the caller, `dbus.rs::run_health_checks`, which remembers the last
    /// reading + timestamp). This rate — not the lifetime total — is what
    /// severity is based on; a raw delta without normalizing for elapsed
    /// time would still be misleading since Doctor is checked on-demand,
    /// not on a fixed interval.
    ///
    /// Why this exists at all: `Metrics::underruns` counts every sample a
    /// route's playback buffer wanted but didn't have, and per `pw.rs`'s
    /// own documented design, a route counts these the same way whether
    /// the gap is a real buffer underrun *or* a group that simply has no
    /// app audio routed into it right now (silence correctly played as
    /// silence — inaudible, not a glitch) but received real audio at some
    /// earlier point. Confirmed live: a fresh daemon accumulated 180k+
    /// "underruns" within minutes of ordinary routing changes (moving a
    /// stream from one group to another leaves the vacated group's route
    /// silently "underrunning" forever) with zero audible problem — the
    /// lifetime counter alone makes this check permanently red for the
    /// rest of the daemon's uptime regardless of whether anything is
    /// actually wrong *now*.
    pub underrun_rate_per_sec: f32,
    pub total_outputs: usize,
    pub available_outputs: usize,
    pub routes_pointing_at_unavailable_outputs: usize,
}

/// Each check returns OK/Warning/Error plus an explanation (spec's own
/// required shape) — pure function of `HealthInputs`, so it's testable
/// without a live daemon.
pub fn run_checks(inputs: &HealthInputs) -> Vec<HealthCheck> {
    let mut checks = Vec::new();

    checks.push(HealthCheck {
        name: "PipeWire connection".to_owned(),
        severity: Severity::Ok,
        message: "Connected — this response only exists because the daemon has a live PipeWire session.".to_owned(),
    });

    let missing: Vec<u32> = inputs.configured_group_ids.iter().filter(|id| !inputs.live_group_ids.contains(id)).copied().collect();
    checks.push(if missing.is_empty() {
        HealthCheck { name: "Groups".to_owned(), severity: Severity::Ok, message: format!("{} group(s), all present in the PipeWire graph.", inputs.configured_group_ids.len()) }
    } else {
        HealthCheck {
            name: "Groups".to_owned(),
            severity: Severity::Error,
            message: format!("{} group(s) missing their PipeWire node: {missing:?}. Try the resync repair.", missing.len()),
        }
    });

    checks.push(if inputs.buffer_frames > 0 {
        HealthCheck { name: "Audio buffer".to_owned(), severity: Severity::Ok, message: format!("{} frames/buffer.", inputs.buffer_frames) }
    } else {
        HealthCheck { name: "Audio buffer".to_owned(), severity: Severity::Warning, message: "No buffer size reported yet — nothing has processed audio.".to_owned() }
    });

    // Thresholds are per-second *rates*, not lifetime totals — see
    // `HealthInputs::underrun_rate_per_sec`'s doc comment for why a
    // lifetime count is the wrong signal for "is this happening now".
    // 512 frames @ 48kHz stereo is ~187 sample-slots/sec of theoretical
    // max legitimate jitter from buffer-size rounding alone; a few/sec is
    // unremarkable, tens/sec sustained is worth a look, hundreds/sec is a
    // route that's continuously starved.
    checks.push(if inputs.underrun_rate_per_sec < 1.0 {
        HealthCheck {
            name: "Underruns".to_owned(),
            severity: Severity::Ok,
            message: format!("None recently ({} lifetime, from routing changes/startup — not currently glitching).", inputs.underrun_count),
        }
    } else if inputs.underrun_rate_per_sec < 50.0 {
        HealthCheck {
            name: "Underruns".to_owned(),
            severity: Severity::Warning,
            message: format!("~{:.0}/sec recently — occasional glitches possible.", inputs.underrun_rate_per_sec),
        }
    } else {
        HealthCheck {
            name: "Underruns".to_owned(),
            severity: Severity::Error,
            message: format!("~{:.0}/sec recently — audible glitching likely.", inputs.underrun_rate_per_sec),
        }
    });

    checks.push(if inputs.total_outputs == 0 {
        HealthCheck { name: "Outputs".to_owned(), severity: Severity::Warning, message: "No PipeWire outputs discovered.".to_owned() }
    } else if inputs.available_outputs < inputs.total_outputs {
        HealthCheck {
            name: "Outputs".to_owned(),
            severity: Severity::Warning,
            message: format!("{}/{} outputs currently available.", inputs.available_outputs, inputs.total_outputs),
        }
    } else {
        HealthCheck { name: "Outputs".to_owned(), severity: Severity::Ok, message: format!("{} output(s), all available.", inputs.total_outputs) }
    });

    checks.push(if inputs.routes_pointing_at_unavailable_outputs == 0 {
        HealthCheck { name: "Output routes".to_owned(), severity: Severity::Ok, message: "Every configured route points at a currently-available output.".to_owned() }
    } else {
        HealthCheck {
            name: "Output routes".to_owned(),
            severity: Severity::Warning,
            message: format!("{} route(s) waiting on a currently-unavailable output — not lost, just disconnected.", inputs.routes_pointing_at_unavailable_outputs),
        }
    });

    checks.push(HealthCheck { name: "D-Bus".to_owned(), severity: Severity::Ok, message: "Responsive — this itself is a D-Bus reply.".to_owned() });

    checks
}

pub fn overall_score(checks: &[HealthCheck]) -> Severity {
    if checks.iter().any(|c| c.severity == Severity::Error) {
        Severity::Error
    } else if checks.iter().any(|c| c.severity == Severity::Warning) {
        Severity::Warning
    } else {
        Severity::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_healthy_scores_ok() {
        let inputs = HealthInputs {
            configured_group_ids: vec![0, 1],
            live_group_ids: vec![0, 1],
            buffer_frames: 512,
            underrun_count: 0,
            underrun_rate_per_sec: 0.0,
            total_outputs: 2,
            available_outputs: 2,
            routes_pointing_at_unavailable_outputs: 0,
        };
        assert_eq!(overall_score(&run_checks(&inputs)), Severity::Ok);
    }

    #[test]
    fn missing_group_is_an_error() {
        let inputs = HealthInputs { configured_group_ids: vec![0, 1], live_group_ids: vec![0], ..Default::default() };
        assert_eq!(overall_score(&run_checks(&inputs)), Severity::Error);
    }

    #[test]
    fn high_underrun_rate_is_an_error_low_rate_is_a_warning() {
        let mut inputs = HealthInputs { underrun_rate_per_sec: 200.0, ..Default::default() };
        assert_eq!(overall_score(&run_checks(&inputs)), Severity::Error);
        inputs.underrun_rate_per_sec = 5.0;
        assert_eq!(overall_score(&run_checks(&inputs)), Severity::Warning);
    }

    /// The exact scenario found live: a large *lifetime* underrun total
    /// (from a group briefly going quiet after a routing change) must not
    /// alone trigger Error — only a currently-elevated *rate* should.
    #[test]
    fn large_lifetime_count_with_zero_recent_rate_is_ok() {
        let inputs = HealthInputs {
            configured_group_ids: vec![0],
            live_group_ids: vec![0],
            buffer_frames: 512,
            underrun_count: 180_000,
            underrun_rate_per_sec: 0.0,
            total_outputs: 1,
            available_outputs: 1,
            routes_pointing_at_unavailable_outputs: 0,
        };
        assert_eq!(overall_score(&run_checks(&inputs)), Severity::Ok);
    }

    #[test]
    fn unavailable_output_is_a_warning_not_an_error() {
        let inputs = HealthInputs { total_outputs: 2, available_outputs: 1, ..Default::default() };
        assert_eq!(overall_score(&run_checks(&inputs)), Severity::Warning);
    }
}
