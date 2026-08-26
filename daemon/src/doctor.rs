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
    pub underrun_count: u64,
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

    checks.push(if inputs.underrun_count == 0 {
        HealthCheck { name: "Underruns".to_owned(), severity: Severity::Ok, message: "None detected.".to_owned() }
    } else if inputs.underrun_count < 100 {
        HealthCheck { name: "Underruns".to_owned(), severity: Severity::Warning, message: format!("{} detected — occasional glitches possible.", inputs.underrun_count) }
    } else {
        HealthCheck { name: "Underruns".to_owned(), severity: Severity::Error, message: format!("{} detected — audible glitching likely.", inputs.underrun_count) }
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
    fn high_underruns_is_an_error_low_is_a_warning() {
        let mut inputs = HealthInputs { underrun_count: 500, ..Default::default() };
        assert_eq!(overall_score(&run_checks(&inputs)), Severity::Error);
        inputs.underrun_count = 5;
        assert_eq!(overall_score(&run_checks(&inputs)), Severity::Warning);
    }

    #[test]
    fn unavailable_output_is_a_warning_not_an_error() {
        let inputs = HealthInputs { total_outputs: 2, available_outputs: 1, ..Default::default() };
        assert_eq!(overall_score(&run_checks(&inputs)), Severity::Warning);
    }
}
