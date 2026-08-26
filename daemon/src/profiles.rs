//! Per-App Audio Profiles (NewImprovenments/03-per-app-audio-profiles.md):
//! persist a routing/volume profile per application, restored automatically
//! whenever that application's stream reappears.
//!
//! Scope decision (documented, not silent): the spec's profile also lists
//! EQ/HPF/LPF/compressor/gate, but this codebase's DSP chain — by design,
//! stated plainly in PROJECT_SPEC.md Section 7.2 and reinforced by the
//! existing Inputs tab's own comment — runs **per group**, not per
//! individual app stream. Building genuine per-app DSP would mean giving
//! every app stream its own capture/processing chain instead of mixing into
//! its group's shared one: a real architectural change well beyond this
//! feature, not a profile-shape decision. What a profile *can* honestly
//! cover on top of today's architecture — and what's implemented here —
//! is exactly what's independent of the group's shared chain: which group
//! and output an app lands in, and its own PipeWire stream volume (already
//! a genuinely per-stream property, the same one `dbus.rs::set_stream_muted`
//! already controls via `wpctl`). That still delivers the spec's own
//! success criterion ("Spotify, Discord, games, etc. always return to the
//! user's preferred audio configuration") for routing and level, just not
//! per-app EQ shaping. Profile inheritance (an advanced feature building on
//! the full DSP shape) is skipped for the same reason.

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
