//! Persisted config: groups, routing, DSP params. TOML under
//! `~/.config/mtxpanel/config.toml` (PROJECT_SPEC.md Section 10, OPEN item
//! resolved here — plain TOML is enough for a per-user settings file and
//! keeps this human-editable/debuggable without a DB dependency).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::group::Group;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub groups: Vec<Group>,
}

impl Config {
    pub fn path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let home = std::env::var("HOME").expect("HOME must be set");
                PathBuf::from(home).join(".config")
            });
        base.join("mtxpanel").join("config.toml")
    }

    pub fn load_or_default() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(raw) => toml::from_str(&raw).unwrap_or_else(|err| {
                tracing::warn!(%err, "config file invalid, starting from defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let raw = toml::to_string_pretty(self).expect("Config always serializes");
        std::fs::write(path, raw)
    }
}
