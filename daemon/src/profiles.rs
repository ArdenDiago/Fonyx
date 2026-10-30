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
    /// Per-site groups for an app whose streams carry a site in their
    /// title (every tab of a browser shares one app id). Moving a YouTube
    /// tab remembers YouTube's group here, and the app-level
    /// `preferred_group` stays as it was for every other tab.
    #[serde(default)]
    pub site_groups: Vec<SiteGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SiteGroup {
    pub site: String,
    pub group: u32,
}

impl AppProfile {
    pub fn new(app_id: AppId, label: String) -> Self {
        Self { app_id, label, volume: 1.0, preferred_group: None, preferred_output: None, site_groups: Vec::new() }
    }

    /// The group remembered for `site`, matched case-insensitively.
    pub fn group_for_site(&self, site: &str) -> Option<u32> {
        self.site_groups.iter().find(|s| s.site.eq_ignore_ascii_case(site)).map(|s| s.group)
    }

    /// A manual move of a stream: with a site, only that site remembers
    /// the group; without one, the whole app does.
    pub fn remember(&mut self, site: &str, group: u32) {
        if site.is_empty() {
            self.preferred_group = Some(group);
            return;
        }
        self.site_groups.retain(|s| !s.site.eq_ignore_ascii_case(site));
        self.site_groups.push(SiteGroup { site: site.to_owned(), group });
    }

    /// Where a new stream of this app goes: its site's group if one is
    /// remembered, else the app's.
    pub fn group_for(&self, site: &str) -> Option<u32> {
        (!site.is_empty()).then(|| self.group_for_site(site)).flatten().or(self.preferred_group)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_move_is_remembered_for_that_site_only() {
        let mut p = AppProfile::new("appname:Firefox".to_owned(), "Firefox".to_owned());
        p.remember("", 1);
        p.remember("YouTube", 3);
        assert_eq!(p.group_for("youtube"), Some(3));
        assert_eq!(p.group_for("Twitch"), Some(1), "other sites keep the app's group");
        assert_eq!(p.group_for(""), Some(1));
        p.remember("YouTube", 4);
        assert_eq!(p.site_groups.len(), 1, "replaced, not duplicated");
        assert_eq!(p.group_for("YouTube"), Some(4));
        assert_eq!(p.preferred_group, Some(1));
    }
}
