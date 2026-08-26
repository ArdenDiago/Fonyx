//! Audio Scenes (NewImprovenments/01-audio-scenes.md): one-click, persistent
//! audio configurations capturing routing, levels, mute/solo, and DSP
//! settings. `Scene` reuses `Group` wholesale for the DSP/routing snapshot
//! rather than inventing a parallel "GroupState" shape — `Group` already
//! *is* that shape (gain, EQ, filters, compressor, gate, output_routes).
//!
//! Applications are resolved by stable identity (`identity::AppId`), never
//! a transient PipeWire node id (spec's explicit "Daemon Design"
//! instruction) — `stream_assignments` pairs an app id with the group it
//! should land in, applied to whichever live stream currently matches that
//! app id. An app not currently running is simply not matched; activation
//! stays "atomically apply what resolves now", and `Engine` separately
//! remembers unresolved assignments to retry once that app starts (the
//! "Recovery" section's explicit ask), the same way `pending_output_routes`
//! already retries a route whose target device isn't live yet.

use serde::{Deserialize, Serialize};

use crate::group::Group;
use crate::identity::AppId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub master_gain: f32,
    /// Per-group DSP/routing snapshot, matched back onto a live group by
    /// `id` on activation. A group deleted since capture is skipped and
    /// reported as an unresolved target rather than resurrected.
    pub groups: Vec<Group>,
    /// (app_id, group_id) — which app should land in which group.
    pub stream_assignments: Vec<(AppId, u32)>,
}

/// What activating a scene actually managed to do — surfaced to the UI as
/// the "unresolved-target indicator" the spec's UI section asks for,
/// instead of a bare success/failure bool.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SceneActivationReport {
    pub applied_groups: Vec<u32>,
    pub missing_groups: Vec<u32>,
    pub applied_streams: Vec<AppId>,
    /// Apps not currently running — remembered by the caller (`Engine`) and
    /// retried automatically once they start.
    pub pending_streams: Vec<AppId>,
}

impl SceneActivationReport {
    pub fn is_fully_resolved(&self) -> bool {
        self.missing_groups.is_empty() && self.pending_streams.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fully_resolved_report_has_no_gaps() {
        let mut r = SceneActivationReport { applied_groups: vec![1], ..Default::default() };
        assert!(r.is_fully_resolved());
        r.pending_streams.push("appid:discord".to_owned());
        assert!(!r.is_fully_resolved());
    }
}
