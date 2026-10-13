//! Persisted config: groups, routing, DSP params. TOML under
//! `~/.config/fonyx/config.toml` (PROJECT_SPEC.md Section 10, OPEN item
//! resolved here — plain TOML is enough for a per-user settings file and
//! keeps this human-editable/debuggable without a DB dependency).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ducking::DuckingRule;
use crate::group::{Group, GroupOutputRoute};
use crate::profiles::AppProfile;
use crate::rules::Rule;
use crate::scenes::Scene;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub groups: Vec<Group>,
    /// Master fader (Section 7.4): a single multiplier applied on top of
    /// every group's own gain, controlling overall output level.
    #[serde(default = "unity")]
    pub master_gain: f32,
    /// Ceiling on automatic/manual output delay compensation
    /// (MTX_MULTI_OUTPUT_DELAY_COMPENSATION.md §12) — persisted (unlike the
    /// per-output *estimated* latency, which §24 explicitly says never to
    /// persist) since it's a user preference, not a live measurement.
    #[serde(default = "default_max_compensation_ms")]
    pub max_compensation_ms: f32,
    #[serde(default)]
    pub scenes: Vec<Scene>,
    #[serde(default)]
    pub profiles: Vec<AppProfile>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub ducking_rules: Vec<DuckingRule>,
    /// Settings tab (PROJECT_SPEC.md Section 7.5) "audio device selection":
    /// when set, a group with no explicit output routes targets this
    /// device instead of bare PipeWire autoconnect — see
    /// `Engine::sync_group_routes`'s `wanted.is_empty()` branch. A group
    /// with its own explicit routes (Outputs tab) is unaffected; this only
    /// covers the common "never touched the Outputs tab" case.
    #[serde(default)]
    pub preferred_default_output: Option<crate::identity::OutputId>,
    /// Settings tab "buffer size": the capture→playback ring buffer's
    /// warm-up cushion (`create_route_node`'s `PRIME_THRESHOLD`) in
    /// milliseconds of stereo audio — see `pw.rs`'s doc comment on that
    /// constant for why a bigger cushion trades a little startup latency
    /// for headroom against scheduling jitter. Only applied to routes
    /// created after the change (existing ones keep whatever cushion they
    /// already warmed up to); the UI says so.
    #[serde(default = "default_buffer_ms")]
    pub buffer_ms: f32,
    /// The Master bus (Outputs tab "Master" routing target): every group's
    /// fully-processed signal (post its own gain/EQ/dynamics/plugins, post
    /// `master_gain`) summed together and sent to whichever outputs are
    /// listed here — same route shape (gain/enable/sync) a group's own
    /// `output_routes` use, since the per-route behavior is identical; only
    /// the source signal differs. Empty means the master bus plays nowhere
    /// (no implicit-default fallback the way a routeless group has, since
    /// this is an opt-in feature nobody has to touch).
    #[serde(default)]
    pub master_output_routes: Vec<GroupOutputRoute>,
    /// Outputs marked as cue (headphones/IEM): the Cue group routes to
    /// exactly these, and they are where the background gets ducked while
    /// something is cued.
    #[serde(default)]
    pub cue_outputs: Vec<crate::identity::OutputId>,
    /// Linear level the rest of the mix drops to on cue outputs while the
    /// Cue group is playing (0.2 = 20%).
    #[serde(default = "default_cue_duck_level")]
    pub cue_duck_level: f32,
}

fn default_cue_duck_level() -> f32 {
    0.2
}

fn default_buffer_ms() -> f32 {
    85.0
}

fn unity() -> f32 {
    1.0
}

fn default_max_compensation_ms() -> f32 {
    150.0
}

impl Default for Config {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            master_gain: 1.0,
            max_compensation_ms: default_max_compensation_ms(),
            scenes: Vec::new(),
            profiles: Vec::new(),
            rules: Vec::new(),
            ducking_rules: Vec::new(),
            preferred_default_output: None,
            buffer_ms: default_buffer_ms(),
            master_output_routes: Vec::new(),
            cue_outputs: Vec::new(),
            cue_duck_level: default_cue_duck_level(),
        }
    }
}

/// The config directory's names before each rename, newest first.
const OLD_DIR_NAMES: [&str; 2] = ["sonicforge", "mtxpanel"];

/// Moves a config directory left by the old name to `new_dir`, so an
/// upgrade keeps its groups, scenes and rules. Never overwrites: if
/// `new_dir` already exists the old one is left where it is.
fn adopt_old_dir(old_dir: &Path, new_dir: &Path) {
    if new_dir.exists() || !old_dir.is_dir() {
        return;
    }
    match std::fs::rename(old_dir, new_dir) {
        Ok(()) => tracing::info!(from = %old_dir.display(), to = %new_dir.display(), "moved config from the old directory name"),
        Err(err) => tracing::warn!(%err, from = %old_dir.display(), "could not move the old config directory; starting without it"),
    }
}

impl Config {
    pub fn path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let home = std::env::var("HOME").expect("HOME must be set");
                PathBuf::from(home).join(".config")
            });
        base.join("fonyx").join("config.toml")
    }

    pub fn load_or_default() -> Self {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            for old in OLD_DIR_NAMES {
                adopt_old_dir(&dir.with_file_name(old), dir);
            }
        }
        match std::fs::read_to_string(&path) {
            Ok(raw) => toml::from_str(&raw).unwrap_or_else(|err| {
                tracing::warn!(%err, "config file invalid, starting from defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Atomic write (NewImprovenments/05-crash-safe-audio-sessions.md's
    /// explicit instruction): write to a sibling temp file, `fsync` it, then
    /// `rename` into place. A crash or power loss mid-write can never leave
    /// `config.toml` truncated/corrupt this way — `rename` within the same
    /// directory is a single atomic filesystem operation, and the reader
    /// (`load_or_default`) only ever sees the old file or the fully-written
    /// new one, never a partial one.
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        let dir = path.parent().expect("config path always has a parent");
        std::fs::create_dir_all(dir)?;
        let raw = toml::to_string_pretty(self).expect("Config always serializes");

        let tmp_path = dir.join(format!(".config.toml.{}.tmp", std::process::id()));
        let mut file = std::fs::File::create(&tmp_path)?;
        std::io::Write::write_all(&mut file, raw.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp_path, &path)
    }

    /// Makes sure the always-on catch-all "Default" group exists (created
    /// once, on first run) — the primary bus every app stream lands in
    /// unless the user routes it elsewhere. Returns `true` if it had to
    /// create one, so the caller knows to persist. A no-op on every later
    /// startup once that group is already in the config.
    pub fn ensure_default_group(&mut self) -> bool {
        if self.groups.iter().any(|g| g.protected) {
            return false;
        }
        let id = self.groups.iter().map(|g| g.id).max().map_or(0, |max| max + 1);
        let mut group = Group::new(id, "Default");
        group.protected = true;
        self.groups.insert(0, group);
        true
    }

    /// Clears profiles' preferred group when that group is gone (deleted
    /// before `delete_group` started doing this itself). Returns whether
    /// anything changed, so the caller knows to persist.
    pub fn forget_missing_profile_groups(&mut self) -> bool {
        let ids: Vec<u32> = self.groups.iter().map(|g| g.id).collect();
        let mut changed = false;
        for p in self.profiles.iter_mut() {
            if p.preferred_group.is_some_and(|g| !ids.contains(&g)) {
                p.preferred_group = None;
                changed = true;
            }
        }
        changed
    }

    /// Same idea as `ensure_default_group`, for the permanent Cue group.
    /// Adopts a group the old UI created by name so its streams stay put.
    pub fn ensure_cue_group(&mut self) -> bool {
        if self.groups.iter().any(|g| g.cue) {
            return false;
        }
        if let Some(g) = self.groups.iter_mut().find(|g| g.name == "Cue" && !g.protected) {
            g.cue = true;
            g.gain = 1.0; // cue always plays at full level; it has no fader
        } else {
            let id = self.groups.iter().map(|g| g.id).max().map_or(0, |max| max + 1);
            let mut group = Group::new(id, "Cue");
            group.cue = true;
            self.groups.push(group);
        }
        self.sync_cue_routes();
        true
    }

    /// Whether `child` may feed into `parent` (see `Group::feeds_group`).
    pub fn check_feed(&self, child: u32, parent: u32) -> Result<(), String> {
        let find = |id| self.groups.iter().find(|g| g.id == id).ok_or(format!("no group {id}"));
        let (c, p) = (find(child)?, find(parent)?);
        if child == parent {
            return Err("a group can't feed itself".to_owned());
        }
        if c.cue || p.cue || c.protected {
            return Err("Default and Cue can't be sub-groups, and Cue can't be fed".to_owned());
        }
        if p.feeds_group.is_some() {
            return Err(format!("{} is itself a sub-group", p.name));
        }
        if self.groups.iter().any(|g| g.feeds_group == Some(child)) {
            return Err(format!("{} already has sub-groups feeding it", c.name));
        }
        Ok(())
    }

    pub fn cue_group_id(&self) -> Option<u32> {
        self.groups.iter().find(|g| g.cue).map(|g| g.id)
    }

    /// Makes the Cue group's routes exactly `cue_outputs` at unity gain.
    pub fn sync_cue_routes(&mut self) {
        let outputs = self.cue_outputs.clone();
        if let Some(g) = self.groups.iter_mut().find(|g| g.cue) {
            g.output_routes = outputs
                .into_iter()
                .map(|id| GroupOutputRoute::new(id, crate::output::RouteMode::Explicit))
                .collect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_config_dir_is_adopted_but_never_overwrites() {
        let base = std::env::temp_dir().join(format!("fonyx_adopt_test_{}", std::process::id()));
        let (old, new) = (base.join(OLD_DIR_NAMES[0]), base.join("fonyx"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("config.toml"), "old").unwrap();

        adopt_old_dir(&old, &new);
        assert!(!old.exists());
        assert_eq!(std::fs::read_to_string(new.join("config.toml")).unwrap(), "old");

        // A second old directory must not replace what is already there.
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("config.toml"), "stale").unwrap();
        adopt_old_dir(&old, &new);
        assert_eq!(std::fs::read_to_string(new.join("config.toml")).unwrap(), "old");
        assert!(old.exists());

        std::fs::remove_dir_all(&base).unwrap();
    }
    use crate::ducking::DuckingRule;
    use crate::group::GroupOutputRoute;
    use crate::output::RouteMode;
    use crate::plugins::PluginInstance;
    use crate::rules::{Action, Condition, Rule, Trigger};
    use crate::scenes::Scene;

    /// Every field above has `#[serde(default)]`, so an *empty* `Config`
    /// round-trips trivially — that alone would not have caught the real
    /// risk: TOML's data model handles Rust enums with data (`Trigger`,
    /// `Condition`, `Action`) very differently from JSON, and a config with
    /// none of these populated (every fresh install, before a user creates
    /// any Scene/Rule/DuckingRule/plugin) would never exercise that path at
    /// all. This test deliberately populates every new enum-bearing type
    /// this session added, including a `Vec<Action>` (multiple enum
    /// variants in one array) and a `Scene` embedding a full nested `Group`
    /// (which itself contains `GroupOutputRoute`, `PluginInstance`, biquad
    /// EQ bands, etc.) — the worst-case nesting shape, not just a smoke
    /// test with one flat field.
    #[test]
    fn profiles_pointing_at_deleted_groups_are_cleared() {
        let mut cfg = Config { groups: vec![Group::new(4, "Default")], ..Default::default() };
        let mut gone = crate::profiles::AppProfile::new("appid:a".to_owned(), "A".to_owned());
        gone.preferred_group = Some(11);
        let mut kept = crate::profiles::AppProfile::new("appid:b".to_owned(), "B".to_owned());
        kept.preferred_group = Some(4);
        cfg.profiles = vec![gone, kept];
        assert!(cfg.forget_missing_profile_groups());
        assert_eq!(cfg.profiles[0].preferred_group, None);
        assert_eq!(cfg.profiles[1].preferred_group, Some(4));
        assert!(!cfg.forget_missing_profile_groups(), "second pass has nothing to do");
    }

    #[test]
    fn config_with_every_new_feature_populated_round_trips_through_toml() {
        let mut group = Group::new(0, "Music");
        group.output_routes.push(GroupOutputRoute::new("serial:abc".to_owned(), RouteMode::Explicit));
        group.plugins.push(PluginInstance { plugin_type: "soft_clipper".to_owned(), enabled: true, params: vec![("drive".to_owned(), 2.5)] });

        let cfg = Config {
            groups: vec![group.clone()],
            master_gain: 0.8,
            max_compensation_ms: 150.0,
            scenes: vec![Scene {
                id: 0,
                name: "Gaming".to_owned(),
                description: "test".to_owned(),
                master_gain: 1.0,
                groups: vec![group],
                stream_assignments: vec![("appid:discord".to_owned(), 0)],
                stream_placements: vec![(42, "appid:discord".to_owned(), 0)],
            }],
            profiles: vec![AppProfile { app_id: "appid:spotify".to_owned(), label: "Spotify".to_owned(), volume: 0.7, preferred_group: Some(0), preferred_output: Some("serial:abc".to_owned()) }],
            rules: vec![
                Rule {
                    id: 0,
                    name: "Discord to voice".to_owned(),
                    enabled: true,
                    priority: 5,
                    cooldown_ms: 2000,
                    trigger: Trigger::AppStarted { app_id: Some("appid:discord".to_owned()), service: None },
                    conditions: vec![Condition::OutputAvailable("serial:abc".to_owned())],
                    actions: vec![Action::RouteTriggeringAppToGroup { group_id: 0 }, Action::ApplyProfileForTriggeringApp],
                },
                Rule {
                    id: 1,
                    name: "Headset connected".to_owned(),
                    enabled: false,
                    priority: 0,
                    cooldown_ms: 5000,
                    trigger: Trigger::DeviceConnected { output_id: None },
                    conditions: vec![],
                    actions: vec![Action::RouteGroupToOutput { group_id: 0, output_id: "serial:abc".to_owned(), gain: 1.0 }, Action::ActivateScene { scene_id: 0 }],
                },
            ],
            ducking_rules: vec![DuckingRule { id: 0, enabled: true, trigger_group: 1, target_groups: vec![0, 2], reduction_db: 12.0, attack_ms: 20.0, hold_ms: 100.0, release_ms: 300.0, threshold_db: -40.0 }],
            preferred_default_output: Some("serial:abc".to_owned()),
            buffer_ms: 85.0,
            master_output_routes: vec![GroupOutputRoute::new("serial:abc".to_owned(), RouteMode::Explicit)],
            cue_outputs: vec!["serial:abc".to_owned()],
            cue_duck_level: 0.2,
        };

        let toml_str = toml::to_string_pretty(&cfg).expect("serialize");
        let round_tripped: Config = toml::from_str(&toml_str).unwrap_or_else(|err| panic!("TOML round-trip failed: {err}\n\n{toml_str}"));

        assert_eq!(round_tripped.rules.len(), 2);
        assert_eq!(round_tripped.scenes.len(), 1);
        assert_eq!(round_tripped.scenes[0].groups[0].plugins.len(), 1);
        assert!(matches!(round_tripped.rules[0].trigger, Trigger::AppStarted { app_id: Some(ref id), .. } if id == "appid:discord"));
        assert_eq!(round_tripped.rules[1].actions.len(), 2);
        assert_eq!(round_tripped.ducking_rules[0].target_groups, vec![0, 2]);
        assert_eq!(round_tripped.preferred_default_output.as_deref(), Some("serial:abc"));
        assert_eq!(round_tripped.buffer_ms, 85.0);
        assert_eq!(round_tripped.cue_outputs, vec!["serial:abc".to_owned()]);
    }

    #[test]
    fn cue_group_is_created_once_and_routes_follow_cue_outputs() {
        let mut cfg = Config::default();
        cfg.ensure_default_group();
        assert!(cfg.ensure_cue_group());
        assert!(!cfg.ensure_cue_group(), "second call must be a no-op");
        let cue = cfg.cue_group_id().unwrap();
        assert!(cfg.groups.iter().filter(|g| g.cue).count() == 1);
        assert!(cfg.groups.iter().find(|g| g.id == cue).unwrap().output_routes.is_empty());

        cfg.cue_outputs = vec!["serial:iem".to_owned()];
        cfg.sync_cue_routes();
        let routes = &cfg.groups.iter().find(|g| g.id == cue).unwrap().output_routes;
        assert_eq!(routes.iter().map(|r| r.output_id.as_str()).collect::<Vec<_>>(), vec!["serial:iem"]);

        // An old UI-created "Cue" group is adopted, not duplicated.
        let mut old = Config::default();
        old.groups.push(Group::new(7, "Cue"));
        old.ensure_cue_group();
        assert_eq!(old.cue_group_id(), Some(7));
        assert_eq!(old.groups.len(), 1);
    }

    #[test]
    fn check_feed_allows_one_level_only() {
        let mut cfg = Config::default();
        cfg.ensure_default_group();
        cfg.ensure_cue_group();
        let (default, cue) = (cfg.groups[0].id, cfg.cue_group_id().unwrap());
        cfg.groups.push(crate::group::Group::new(10, "Music"));
        cfg.groups.push(crate::group::Group::new(11, "Spotify"));
        cfg.groups.push(crate::group::Group::new(12, "Other"));
        assert!(cfg.check_feed(11, 10).is_ok());
        assert!(cfg.check_feed(11, default).is_ok(), "an app strip can feed Default");
        assert!(cfg.check_feed(11, 11).is_err());
        assert!(cfg.check_feed(11, cue).is_err());
        assert!(cfg.check_feed(default, 10).is_err());
        cfg.groups.iter_mut().find(|g| g.id == 11).unwrap().feeds_group = Some(10);
        assert!(cfg.check_feed(12, 11).is_err(), "no feeding a sub-group");
        assert!(cfg.check_feed(10, 12).is_err(), "a parent can't become a sub-group");
    }
}
