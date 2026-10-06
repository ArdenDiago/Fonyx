//! Turning what someone typed into what the daemon wants: names and aliases
//! into ids, and `75%` / `-6dB` into linear gain.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The faders' own ceiling in the UI (+12 dB).
const MAX_GAIN: f32 = 4.0;

/// One thing a query can match: its id, then the names it answers to
/// (`names[0]` is the one shown to the user).
pub struct Cand {
    pub id: String,
    pub names: Vec<String>,
}

/// Indexes of the candidates `query` means. Tries, in order: exact id, exact
/// name, name prefix, name substring (names compare case-insensitively) and
/// stops at the first tier that matches anything. Several exact matches are
/// all returned (every Firefox stream is "Firefox"); a partial match must
/// point at one name only.
pub fn resolve(kind: &str, query: &str, cands: &[Cand]) -> Result<Vec<usize>, String> {
    let q = query.to_lowercase();
    let matching = |test: &dyn Fn(&Cand) -> bool| -> Vec<usize> { (0..cands.len()).filter(|&i| test(&cands[i])).collect() };

    let by_id = matching(&|c| c.id == query);
    if !by_id.is_empty() {
        return Ok(by_id);
    }
    let exact = matching(&|c| c.names.iter().any(|n| n.to_lowercase() == q));
    if !exact.is_empty() {
        return Ok(exact);
    }
    for partial in [
        matching(&|c| c.names.iter().any(|n| n.to_lowercase().starts_with(&q))),
        matching(&|c| c.names.iter().any(|n| n.to_lowercase().contains(&q))),
    ] {
        let Some(&first) = partial.first() else { continue };
        if partial.iter().all(|&i| cands[i].names[0] == cands[first].names[0]) {
            return Ok(partial);
        }
        let list: Vec<String> = partial.iter().map(|&i| format!("{} ({})", cands[i].names[0], cands[i].id)).collect();
        return Err(format!("\"{query}\" matches several {kind}s: {}. Use the id.", list.join(", ")));
    }
    Err(format!("no {kind} matches \"{query}\" (see: sonicforge {kind} ls)"))
}

/// `75%`, `-6dB` or a bare linear multiplier (`0.75`) as linear gain.
pub fn level(s: &str) -> Result<f32, String> {
    let lower = s.trim().to_lowercase();
    let parse = |n: &str| n.trim().parse::<f32>().ok().filter(|v| v.is_finite()).ok_or_else(|| format!("\"{s}\" is not a level (try 75%, -6dB or 0.75)"));
    let gain = if let Some(n) = lower.strip_suffix('%') {
        parse(n)? / 100.0
    } else if let Some(n) = lower.strip_suffix("db") {
        10f32.powf(parse(n)? / 20.0)
    } else {
        parse(&lower)?
    };
    if gain < 0.0 {
        return Err(format!("\"{s}\" is below silence"));
    }
    // A bare `75` meant as 75% would be +37 dB into someone's ears.
    if gain > MAX_GAIN {
        return Err(format!("\"{s}\" is {:+.1} dB, above the +12 dB ceiling. For a percentage write {s}%.", gain_db(gain)));
    }
    Ok(gain)
}

pub fn gain_db(gain: f32) -> f32 {
    20.0 * gain.log10()
}

/// A finite number: `nan` and `inf` parse as `f32` but must never reach a filter.
pub fn num(s: &str) -> Result<f32, String> {
    s.parse::<f32>().ok().filter(|v| v.is_finite()).ok_or_else(|| format!("\"{s}\" is not a number"))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alias {
    /// `input`, `output` or `group`.
    pub kind: String,
    /// What the daemon knows it by: an app id, an output id or a group id.
    pub target: String,
}

/// The user's own short names, kept beside the daemon's config. They are a
/// CLI convenience only; the daemon never sees them.
#[derive(Default)]
pub struct Aliases(pub BTreeMap<String, Alias>);

impl Aliases {
    fn path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
        base.join("sonicforge").join("aliases.toml")
    }

    pub fn load() -> Result<Self, String> {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map(Self).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        let text = toml::to_string(&self.0).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(path.parent().unwrap()).and_then(|()| std::fs::write(&path, text)).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// `query` itself, or what it stands for when it is an alias of `kind`.
    pub fn expand<'a>(&'a self, kind: &str, query: &'a str) -> &'a str {
        self.0.get(query).filter(|a| a.kind == kind).map_or(query, |a| &a.target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cands(list: &[(&str, &[&str])]) -> Vec<Cand> {
        list.iter().map(|(id, names)| Cand { id: id.to_string(), names: names.iter().map(|n| n.to_string()).collect() }).collect()
    }

    #[test]
    fn resolve_prefers_id_then_exact_then_partial() {
        let c = cands(&[("9", &["firefox"]), ("4", &["Default"]), ("12", &["9"]), ("6", &["Classes"])]);
        assert_eq!(resolve("group", "9", &c), Ok(vec![0]), "an id beats a group that is named like one");
        assert_eq!(resolve("group", "FIREFOX", &c), Ok(vec![0]));
        assert_eq!(resolve("group", "cla", &c), Ok(vec![3]));
        assert_eq!(resolve("group", "fault", &c), Ok(vec![1]));
        assert!(resolve("group", "nope", &c).unwrap_err().contains("no group matches"));
    }

    #[test]
    fn resolve_returns_every_exact_match_but_refuses_a_vague_partial() {
        let c = cands(&[("86", &["Firefox", "WhatsApp", "appname:Firefox"]), ("93", &["Firefox", "YouTube", "appname:Firefox"]), ("95", &["Firewatch", "", "appname:Firewatch"])]);
        assert_eq!(resolve("input", "firefox", &c), Ok(vec![0, 1]));
        assert_eq!(resolve("input", "appname:Firefox", &c), Ok(vec![0, 1]));
        assert_eq!(resolve("input", "you", &c), Ok(vec![1]));
        let err = resolve("input", "fire", &c).unwrap_err();
        assert!(err.contains("several inputs") && err.contains("Firewatch (95)"), "{err}");
    }

    #[test]
    fn level_reads_percent_decibels_and_linear() {
        assert_eq!(level("75%"), Ok(0.75));
        assert_eq!(level("0.5"), Ok(0.5));
        assert!((level("-6dB").unwrap() - 0.501).abs() < 0.001);
        assert!((level("+12db").unwrap() - 3.981).abs() < 0.001);
        assert_eq!(level("0"), Ok(0.0));
        assert!(level("75").unwrap_err().contains("75%"), "a bare 75 is a typo, not +37 dB");
        assert!(level("-10%").is_err());
        assert!(level("nan").is_err());
        assert!(level("loud").is_err());
    }

    #[test]
    fn aliases_only_apply_to_their_own_kind() {
        let mut a = Aliases::default();
        a.0.insert("cans".into(), Alias { kind: "output".into(), target: "serial:abc".into() });
        assert_eq!(a.expand("output", "cans"), "serial:abc");
        assert_eq!(a.expand("group", "cans"), "cans");
        assert_eq!(a.expand("output", "speakers"), "speakers");
    }
}
