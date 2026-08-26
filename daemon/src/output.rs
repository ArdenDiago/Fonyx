//! Output device model and default-output policy
//! (NewImprovenments/MTX_AUDIO_GROUPS_MULTI_OUTPUT.md §4/§5,
//! MTX_MULTI_OUTPUT_DELAY_COMPENSATION.md §4/§6). Pure data + policy logic;
//! PipeWire discovery lives in `pw.rs`'s `on_global` (Sink/Duplex Node +
//! Device globals), DSP lives in `dsp::delay`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::group::GroupOutputRoute;
use crate::identity::{OutputId, OutputKind};

/// A discovered PipeWire output (sink), keyed by `OutputId` (identity.rs) —
/// never by the underlying transient `node_id`, which is only valid while
/// the node currently exists.
#[derive(Debug, Clone)]
pub struct OutputDevice {
    pub id: OutputId,
    pub name: String,
    pub description: String,
    pub kind: OutputKind,
    /// `None` while the device is disconnected (MTX_AUDIO_GROUPS_MULTI_OUTPUT.md
    /// §13 — the route persists, the device just isn't live right now).
    pub node_id: Option<u32>,
    pub is_system_default: bool,
    pub sample_rate: u32,
    pub channels: u32,
    pub latency: OutputLatency,
}

impl OutputDevice {
    pub fn available(&self) -> bool {
        self.node_id.is_some()
    }
}

/// Best-effort latency estimate
/// (MTX_MULTI_OUTPUT_DELAY_COMPENSATION.md §9): PipeWire doesn't reliably
/// expose true end-to-end device latency for every backend (notably
/// Bluetooth), so this distinguishes "we read a real number off the graph"
/// from "unknown" instead of ever inventing one — the spec's explicit
/// instruction not to hard-code arbitrary Bluetooth latency values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputLatency {
    Measured(f32),
    Unknown,
}

impl OutputLatency {
    pub fn ms(&self) -> Option<f32> {
        match self {
            OutputLatency::Measured(ms) => Some(*ms),
            OutputLatency::Unknown => None,
        }
    }
}

/// Whether a route was created by the automatic default-output policy or by
/// explicit user action (§14) — automatic routes get silently updated as
/// devices come and go; explicit ones never do (§5/§15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RouteMode {
    #[default]
    Explicit,
    Automatic,
}

impl RouteMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            RouteMode::Explicit => "explicit",
            RouteMode::Automatic => "automatic",
        }
    }
}

/// Output synchronization mode (MTX_MULTI_OUTPUT_DELAY_COMPENSATION.md §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    #[default]
    Off,
    Automatic,
    Manual,
}

impl SyncMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            SyncMode::Off => "off",
            SyncMode::Automatic => "automatic",
            SyncMode::Manual => "manual",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(SyncMode::Off),
            "automatic" => Some(SyncMode::Automatic),
            "manual" => Some(SyncMode::Manual),
            _ => None,
        }
    }
}

/// Applies the default-output policy (§5): headphones, else speakers, else
/// whatever PipeWire itself currently calls the default sink, else whatever
/// is available. Only ever consulted for `RouteMode::Automatic` routes —
/// callers must never use this to override an explicit route.
///
/// ponytail: written and unit-tested but not yet wired into `pw.rs` to
/// auto-create routes on device connect — this pass's `RouteMode::Automatic`
/// exists as a ready persisted/API value, but a route today is only ever
/// created when the user (or a future rule) adds one explicitly. An empty
/// route list already keeps a group on the original AUTOCONNECT-to-default
/// behavior, which covers the common single-output case. Upgrade path:
/// call this from `Engine::sync_group_routes` once a UI surface exists for
/// "make this an automatic route" (MTX_AUDIO_GROUPS_MULTI_OUTPUT.md's
/// priority 7, after the basic routing this pass ships).
#[allow(dead_code)]
pub fn pick_default_output<'a>(
    outputs: impl Iterator<Item = &'a OutputDevice>,
    system_default_id: Option<&OutputId>,
) -> Option<OutputId> {
    let available: Vec<&OutputDevice> = outputs.filter(|o| o.available()).collect();
    if let Some(headphones) = available.iter().find(|o| o.kind == OutputKind::Headphones) {
        return Some(headphones.id.clone());
    }
    if let Some(speaker) = available.iter().find(|o| o.kind == OutputKind::Speaker) {
        return Some(speaker.id.clone());
    }
    if let Some(default_id) = system_default_id {
        if available.iter().any(|o| &o.id == default_id) {
            return Some(default_id.clone());
        }
    }
    available.first().map(|o| o.id.clone())
}

/// Computes the delay (ms) MTX should actually apply to one route, given
/// its sync mode and — for Automatic — the other routes it shares a group
/// with (MTX_MULTI_OUTPUT_DELAY_COMPENSATION.md §3/§6/§13/§20). Pure
/// function, no PipeWire access, so the synchronization math is fully
/// unit-testable on its own.
pub fn resolve_route_delay_ms(
    route: &GroupOutputRoute,
    siblings: &[GroupOutputRoute],
    outputs: &HashMap<OutputId, OutputDevice>,
    max_compensation_ms: f32,
) -> f32 {
    match route.sync_mode {
        SyncMode::Off => 0.0,
        SyncMode::Manual => route.manual_delay_ms.clamp(0.0, max_compensation_ms),
        SyncMode::Automatic => {
            if !route.sync_included || !route.enabled {
                return 0.0;
            }
            let Some(this_latency) = outputs.get(&route.output_id).and_then(|o| o.latency.ms()) else {
                // Unknown latency: never guess — fail safe to zero added
                // delay instead of fabricating a number (§9/§20).
                return 0.0;
            };
            let reference = siblings
                .iter()
                .filter(|r| r.sync_mode == SyncMode::Automatic && r.sync_included && r.enabled)
                .filter_map(|r| outputs.get(&r.output_id).and_then(|o| o.latency.ms()))
                .fold(this_latency, f32::max);
            (reference - this_latency).clamp(0.0, max_compensation_ms)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: &str, kind: OutputKind, available: bool) -> OutputDevice {
        OutputDevice {
            id: id.to_owned(),
            name: id.to_owned(),
            description: id.to_owned(),
            kind,
            node_id: available.then_some(1),
            is_system_default: false,
            sample_rate: 48_000,
            channels: 2,
            latency: OutputLatency::Unknown,
        }
    }

    #[test]
    fn prefers_headphones_over_speakers() {
        let outputs = vec![dev("speakers", OutputKind::Speaker, true), dev("hp", OutputKind::Headphones, true)];
        assert_eq!(pick_default_output(outputs.iter(), None), Some("hp".to_owned()));
    }

    #[test]
    fn falls_back_to_speakers_without_headphones() {
        let outputs = vec![dev("speakers", OutputKind::Speaker, true), dev("bt", OutputKind::Bluetooth, true)];
        assert_eq!(pick_default_output(outputs.iter(), None), Some("speakers".to_owned()));
    }

    #[test]
    fn ignores_unavailable_headphones() {
        let outputs = vec![dev("speakers", OutputKind::Speaker, true), dev("hp", OutputKind::Headphones, false)];
        assert_eq!(pick_default_output(outputs.iter(), None), Some("speakers".to_owned()));
    }

    #[test]
    fn falls_back_to_system_default_then_anything_available() {
        let outputs = vec![dev("bt", OutputKind::Bluetooth, true)];
        assert_eq!(pick_default_output(outputs.iter(), Some(&"bt".to_owned())), Some("bt".to_owned()));

        let outputs2 = vec![dev("hdmi", OutputKind::Hdmi, true)];
        assert_eq!(pick_default_output(outputs2.iter(), None), Some("hdmi".to_owned()));
    }

    fn dev_with_latency(id: &str, ms: f32) -> OutputDevice {
        OutputDevice { latency: OutputLatency::Measured(ms), ..dev(id, OutputKind::Speaker, true) }
    }

    fn route(output_id: &str, sync_mode: SyncMode) -> GroupOutputRoute {
        GroupOutputRoute { sync_mode, ..GroupOutputRoute::new(output_id.to_owned(), RouteMode::Explicit) }
    }

    #[test]
    fn sync_off_means_no_delay() {
        let route = route("a", SyncMode::Off);
        let outputs = HashMap::from([("a".to_owned(), dev_with_latency("a", 90.0))]);
        assert_eq!(resolve_route_delay_ms(&route, &[], &outputs, 150.0), 0.0);
    }

    #[test]
    fn manual_mode_uses_configured_value_clamped_to_max() {
        let mut r = route("a", SyncMode::Manual);
        r.manual_delay_ms = 70.0;
        assert_eq!(resolve_route_delay_ms(&r, &[], &HashMap::new(), 150.0), 70.0);

        r.manual_delay_ms = 999.0;
        assert_eq!(resolve_route_delay_ms(&r, &[], &HashMap::new(), 150.0), 150.0);
    }

    #[test]
    fn automatic_mode_compensates_against_the_slowest_sibling() {
        let fast = route("fast", SyncMode::Automatic);
        let slow = route("slow", SyncMode::Automatic);
        let outputs = HashMap::from([
            ("fast".to_owned(), dev_with_latency("fast", 20.0)),
            ("slow".to_owned(), dev_with_latency("slow", 90.0)),
        ]);
        let siblings = vec![fast.clone(), slow.clone()];

        assert_eq!(resolve_route_delay_ms(&fast, &siblings, &outputs, 150.0), 70.0);
        assert_eq!(resolve_route_delay_ms(&slow, &siblings, &outputs, 150.0), 0.0);
    }

    #[test]
    fn automatic_mode_ignores_excluded_siblings() {
        let a = route("a", SyncMode::Automatic);
        let mut b = route("b", SyncMode::Automatic);
        b.sync_included = false;
        let outputs = HashMap::from([
            ("a".to_owned(), dev_with_latency("a", 20.0)),
            ("b".to_owned(), dev_with_latency("b", 200.0)), // would dominate if not excluded
        ]);
        let siblings = vec![a.clone(), b];
        assert_eq!(resolve_route_delay_ms(&a, &siblings, &outputs, 150.0), 0.0);
    }

    #[test]
    fn automatic_mode_fails_safe_to_zero_when_latency_unknown() {
        let r = route("a", SyncMode::Automatic);
        let outputs = HashMap::from([("a".to_owned(), dev("a", OutputKind::Bluetooth, true))]); // Unknown latency
        assert_eq!(resolve_route_delay_ms(&r, &[], &outputs, 150.0), 0.0);
    }
}
