//! Per-App Audio Profiles (NewImprovenments/03-per-app-audio-profiles.md):
//! persist a routing/volume profile per application, restored automatically
//! whenever that application's stream reappears.
//!
//! The profile itself holds routing and stream volume. Per-app EQ/HPF/LPF/
//! compressor/gate come from the group layer instead: DSP runs per group,
//! and "Own DSP" (`dbus.rs::create_app_strip`) gives an app a sub-group of
//! its own (`Group::feeds_group`) feeding the group it was in, then points
//! this profile's `preferred_group` at it, so the app's own DSP comes back
//! whenever it restarts. Profile inheritance is skipped.

use serde::{Deserialize, Serialize};

use crate::identity::{AppId, OutputId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppProfile {
    pub app_id: AppId,
    /// Display label (usually the app's own name) — editable so a profile
    /// keyed by a cryptic executable-derived id still shows something
    /// readable.
    pub label: String,
    /// Linear PipeWire stream volume (applied via `wpctl`), independent of
    /// whatever group the app lands in.
    pub volume: f32,
    pub preferred_group: Option<u32>,
    pub preferred_output: Option<OutputId>,
}

impl AppProfile {
    pub fn new(app_id: AppId, label: String) -> Self {
        Self { app_id, label, volume: 1.0, preferred_group: None, preferred_output: None }
    }
}
