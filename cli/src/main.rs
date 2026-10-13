//! `fonyx`: the mixer from the terminal. A thin client: every command
//! is one or a few calls on the daemon's D-Bus interface (docs/dbus.md), or
//! `systemctl --user` for the `system` service commands. No audio logic here.

mod args;
mod out;
mod resolve;
mod system;

use std::collections::HashMap;
use std::fmt::Display;

use clap::Parser;
use serde_json::{json, Value};

use args::*;
use out::{db, n};
use resolve::{gain_db, resolve, Aliases, Cand};

type Res<T = ()> = Result<T, Box<dyn std::error::Error>>;
type GroupRow = (u32, String, f32, Vec<u32>, bool);
type RouteRow = (String, f32, bool, String, String, f32, bool);
type PluginSlot = (u32, String, bool, Vec<String>, Vec<f32>);

/// The daemon's "no such id".
const NONE: u32 = u32::MAX;

struct Ctx {
    d: fonyx_client::DaemonProxyBlocking<'static>,
    json: bool,
    quiet: bool,
    aliases: Aliases,
}

fn main() {
    // Die quietly when piped into `head`, instead of panicking in println!.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let cli = Cli::parse();
    if let Err(err) = run(cli) {
        eprintln!("fonyx: {}", explain(&err.to_string()));
        std::process::exit(1);
    }
}

/// D-Bus error names, said the way a person would.
fn explain(err: &str) -> String {
    if err.contains("ServiceUnknown") || err.contains("NameHasNoOwner") {
        return "the daemon is not running (start it with: fonyx system start)".to_owned();
    }
    if err.contains("UnknownMethod") {
        return "the running daemon is older than this CLI and does not know this command (update it, then: fonyx system restart backend)".to_owned();
    }
    err.rsplit("org.freedesktop.DBus.Error.InvalidArgs: ").next().unwrap_or(err).to_owned()
}

fn run(cli: Cli) -> Res {
    let c = Ctx { d: fonyx_client::connect()?, json: cli.json, quiet: cli.quiet, aliases: Aliases::load()? };
    match cli.cmd {
        Cmd::System(cmd) => system::run(&c, cmd),
        Cmd::Group(cmd) => c.group_cmd(cmd),
        Cmd::Input(cmd) => c.input_cmd(cmd),
        Cmd::Fader(cmd) => c.fader_cmd(cmd),
        Cmd::Dsp(cmd) => c.dsp_cmd(cmd),
        Cmd::Mtx(cmd) => c.mtx_cmd(cmd),
        Cmd::Output(cmd) => c.output_cmd(cmd),
        Cmd::Alias(cmd) => c.alias_cmd(cmd),
        Cmd::Scene(cmd) => c.scene_cmd(cmd),
        Cmd::Rule(cmd) => c.rule_cmd(cmd),
        Cmd::Duck(cmd) => c.duck_cmd(cmd),
        Cmd::Record(cmd) => c.record_cmd(cmd),
    }
}

fn name_of(groups: &[GroupRow], id: u32) -> String {
    groups.iter().find(|g| g.0 == id).map_or_else(|| format!("#{id}"), |g| g.1.clone())
}

fn positive(what: &str, value: Option<f32>) -> Res {
    match value {
        Some(v) if v <= 0.0 => Err(format!("{what} must be above 0").into()),
        _ => Ok(()),
    }
}

fn eq_rows(bands: &[(f32, f32, f32, bool)]) -> Vec<Value> {
    bands.iter().enumerate().map(|(i, b)| json!({"band": i + 1, "freq_hz": n(b.0), "gain_db": n(b.1), "q": n(b.2), "enabled": b.3})).collect()
}

fn filter_obj(f: (f32, f32, bool)) -> Value {
    json!({"freq_hz": n(f.0), "q": n(f.1), "enabled": f.2})
}

fn comp_obj(p: (f32, f32, f32, f32, f32, bool)) -> Value {
    json!({"threshold_db": n(p.0), "ratio": n(p.1), "attack_ms": n(p.2), "release_ms": n(p.3), "makeup_db": n(p.4), "enabled": p.5})
}

fn gate_obj(p: (f32, f32, f32, f32, bool)) -> Value {
    json!({"threshold_db": n(p.0), "attack_ms": n(p.1), "release_ms": n(p.2), "hold_ms": n(p.3), "enabled": p.4})
}

/// "12s", "5m", "3h", "2d": how long ago a unix time was.
fn ago(unix: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    match now.saturating_sub(unix) {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

impl Ctx {
    // ------------------------------------------------------------ output

    /// A one-line confirmation, for people. Scripts get the exit code.
    fn say(&self, msg: impl Display) {
        if !self.quiet && !self.json {
            println!("{msg}");
        }
    }

    fn show(&self, value: Value) {
        out::print(&value, self.json);
    }

    /// Reports a newly made thing; with `--json` its id is the output.
    fn created(&self, what: &str, id: u32) {
        if self.json {
            self.show(json!({"id": id}));
        } else {
            self.say(format!("created {what} (id {id})"));
        }
    }

    /// Prints `build()` once, or keeps reprinting it for `--watch`.
    fn watch(&self, watch: bool, build: impl Fn() -> Res<Value>) -> Res {
        loop {
            let value = build()?;
            match (watch, self.json) {
                (false, _) => {
                    self.show(value);
                    return Ok(());
                }
                (true, true) => println!("{value}"), // one line per tick
                (true, false) => print!("\x1b[H\x1b[2J{}", out::text(&value)),
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }

    // -------------------------------------------------------- resolution

    fn pick(&self, kind: &str, query: &str, cands: &[Cand]) -> Res<Vec<usize>> {
        if query.trim().is_empty() {
            return Err(format!("which {kind}? (see: fonyx {kind} ls)").into());
        }
        Ok(resolve(kind, self.aliases.expand(kind, query), cands)?)
    }

    fn pick_one(&self, kind: &str, query: &str, cands: &[Cand]) -> Res<usize> {
        match self.pick(kind, query, cands)?[..] {
            [one] => Ok(one),
            ref many => {
                let list: Vec<String> = many.iter().map(|&i| format!("{} ({})", cands[i].names[0], cands[i].id)).collect();
                Err(format!("\"{query}\" matches several {kind}s: {}. Use the id.", list.join(", ")).into())
            }
        }
    }

    fn group_in(&self, groups: &[GroupRow], query: &str) -> Res<u32> {
        let cands: Vec<Cand> = groups.iter().map(|g| Cand { id: g.0.to_string(), names: vec![g.1.clone()] }).collect();
        Ok(groups[self.pick_one("group", query, &cands)?].0)
    }

    fn group(&self, query: &str) -> Res<u32> {
        self.group_in(&self.d.list_groups()?, query)
    }

    /// A group, or `None` for the master bus.
    fn bus(&self, query: &str) -> Res<Option<u32>> {
        if query.eq_ignore_ascii_case("master") {
            return Ok(None);
        }
        self.group(query).map(Some)
    }

    /// Every stream the query means: all of an app's streams share its name.
    fn inputs(&self, query: &str) -> Res<Vec<u32>> {
        let streams = self.d.list_streams()?;
        let cands: Vec<Cand> = streams
            .iter()
            .map(|s| Cand { id: s.0.to_string(), names: [&s.1, &s.2, &s.4].into_iter().filter(|x| !x.is_empty()).cloned().collect() })
            .collect();
        Ok(self.pick("input", query, &cands)?.into_iter().map(|i| streams[i].0).collect())
    }

    fn output(&self, query: &str) -> Res<String> {
        let outputs = self.d.list_outputs()?;
        let cands: Vec<Cand> = outputs.iter().map(|o| Cand { id: o.0.clone(), names: vec![o.2.clone(), o.1.clone()] }).collect();
        Ok(outputs[self.pick_one("output", query, &cands)?].0.clone())
    }

    /// Output id to the name people know it by.
    fn output_names(&self) -> Res<HashMap<String, String>> {
        Ok(self.d.list_outputs()?.into_iter().map(|o| (o.0, o.2)).collect())
    }

    fn scene(&self, query: &str) -> Res<u32> {
        let scenes = self.d.list_scenes()?;
        let cands: Vec<Cand> = scenes.iter().map(|s| Cand { id: s.0.to_string(), names: vec![s.1.clone()] }).collect();
        Ok(scenes[self.pick_one("scene", query, &cands)?].0)
    }

    fn rule(&self, query: &str) -> Res<u32> {
        let rules = self.d.list_rules()?;
        let cands: Vec<Cand> = rules.iter().map(|r| Cand { id: r.0.to_string(), names: vec![r.1.clone()] }).collect();
        Ok(rules[self.pick_one("rule", query, &cands)?].0)
    }

    // ------------------------------------------------------------- group

    fn group_cmd(&self, cmd: Group) -> Res {
        let d = &self.d;
        match cmd {
            Group::Ls => {
                let groups = d.list_groups()?;
                let feeds: HashMap<u32, u32> = d.list_group_feeds()?.into_iter().collect();
                let cue = d.get_cue_group_id()?;
                let rows: Vec<Value> = groups
                    .iter()
                    .map(|g| {
                        let kind = if g.4 { json!("default") } else if g.0 == cue { json!("cue") } else { Value::Null };
                        json!({"id": g.0, "name": g.1, "gain": n(g.2), "db": db(g.2), "inputs": g.3.len(), "feeds": feeds.get(&g.0).map(|&p| name_of(&groups, p)), "kind": kind})
                    })
                    .collect();
                self.show(json!(rows));
            }
            Group::Show { group } => {
                let groups = d.list_groups()?;
                let id = self.group_in(&groups, &group)?;
                let g = groups.iter().find(|g| g.0 == id).expect("just resolved");
                let (bands, hp, lp, comp, gate) = d.get_group_details(id)?;
                let pan = d.list_group_pans()?.into_iter().find(|p| p.0 == id).unwrap_or((id, 0.0, false));
                let feeds = d.list_group_feeds()?.into_iter().find(|f| f.0 == id).map(|f| name_of(&groups, f.1));
                let streams = d.list_streams()?;
                let inputs: Vec<&str> = streams.iter().filter(|s| g.3.contains(&s.0)).map(|s| s.1.as_str()).collect();
                self.show(json!({
                    "id": id, "name": g.1, "gain": n(g.2), "db": db(g.2), "pan": n(pan.1), "mono": pan.2,
                    "eq_level_db": n(d.get_eq_level(id)?), "feeds": feeds, "inputs": inputs,
                    "highpass": filter_obj(hp), "lowpass": filter_obj(lp), "compressor": comp_obj(comp), "gate": gate_obj(gate),
                    "eq": eq_rows(&bands), "routes": self.route_rows(None, d.list_group_output_routes(id)?)?, "plugins": self.plugin_rows(id)?,
                }));
            }
            Group::Create { name } => {
                let name = name.trim();
                if name.is_empty() {
                    return Err("a group needs a name".into());
                }
                self.created(&format!("group {name}"), d.create_group(name)?);
            }
            Group::Rm { group } => {
                let groups = d.list_groups()?;
                let id = self.group_in(&groups, &group)?;
                // The daemon refuses these too, but silently.
                if groups.iter().any(|g| g.0 == id && g.4) || id == d.get_cue_group_id()? {
                    return Err("the Default and Cue groups cannot be deleted".into());
                }
                d.delete_group(id)?;
                self.say(format!("deleted group {}", name_of(&groups, id)));
            }
            Group::Rename { group, name } => {
                let groups = d.list_groups()?;
                let id = self.group_in(&groups, &group)?;
                d.rename_group(id, &name)?;
                self.say(format!("renamed group {} to {}", name_of(&groups, id), name.trim()));
            }
            Group::Feed { group, parent } => {
                let groups = d.list_groups()?;
                let id = self.group_in(&groups, &group)?;
                if parent.eq_ignore_ascii_case("none") {
                    d.set_group_feeds(id, NONE)?;
                    self.say(format!("{} plays on its outputs again", name_of(&groups, id)));
                } else {
                    let parent = self.group_in(&groups, &parent)?;
                    d.set_group_feeds(id, parent)?;
                    self.say(format!("{} now feeds {}", name_of(&groups, id), name_of(&groups, parent)));
                }
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------- input

    fn input_cmd(&self, cmd: Input) -> Res {
        let d = &self.d;
        match cmd {
            Input::Ls => {
                let groups = d.list_groups()?;
                let sides: HashMap<u32, String> = d.list_stream_sides()?.into_iter().collect();
                let rows: Vec<Value> = d
                    .list_streams()?
                    .iter()
                    .map(|s| {
                        let group = groups.iter().find(|g| g.3.contains(&s.0)).map(|g| g.1.as_str());
                        let side = sides.get(&s.0).map_or("stereo", String::as_str);
                        json!({"id": s.0, "name": s.1, "service": (!s.2.is_empty()).then_some(&s.2), "playing": s.3, "group": group, "side": side, "app": s.4})
                    })
                    .collect();
                self.show(json!(rows));
            }
            Input::Move { input, group } => {
                let groups = d.list_groups()?;
                let (ids, group_id) = (self.inputs(&input)?, self.group_in(&groups, &group)?);
                for &id in &ids {
                    d.route_stream_to_group(id, group_id)?;
                }
                self.say(format!("moved {} input(s) to {}", ids.len(), name_of(&groups, group_id)));
            }
            Input::Unroute { input } => {
                let ids = self.inputs(&input)?;
                for &id in &ids {
                    d.unroute_stream(id)?;
                }
                self.say(format!("sent {} input(s) back to Default", ids.len()));
            }
            Input::Mute { input } => self.mute_inputs(&input, true)?,
            Input::Unmute { input } => self.mute_inputs(&input, false)?,
            Input::Side { input, side } => {
                let (wire, label) = match side {
                    Side::L => ("L", "the left channel"),
                    Side::R => ("R", "the right channel"),
                    Side::Stereo => ("", "stereo"),
                };
                let ids = self.inputs(&input)?;
                for &id in &ids {
                    d.set_stream_side(id, wire)?;
                }
                self.say(format!("{} input(s) now on {label}", ids.len()));
            }
            Input::Strip { input } => {
                // The daemon moves every stream of the app; one is enough to name it.
                let id = d.create_app_strip(self.inputs(&input)?[0])?;
                if self.json {
                    self.show(json!({"id": id}));
                } else {
                    self.say(format!("the app now has its own group: {} (id {id})", name_of(&d.list_groups()?, id)));
                }
            }
            Input::Profile(Profile::Save { input, label }) => {
                if !d.save_profile_for_stream(self.inputs(&input)?[0], &label)? {
                    return Err("could not save a profile for that input".into());
                }
                self.say(format!("saved profile {label}"));
            }
            Input::Levels { watch } => self.watch(watch, || {
                let peaks: HashMap<u32, f32> = d.get_stream_levels()?.into_iter().collect();
                let rows: Vec<Value> = d.list_streams()?.iter().map(|s| json!({"id": s.0, "name": s.1, "service": (!s.2.is_empty()).then_some(&s.2), "peak_db": db(peaks.get(&s.0).copied().unwrap_or(0.0))})).collect();
                Ok(json!(rows))
            })?,
        }
        Ok(())
    }

    fn mute_inputs(&self, input: &str, muted: bool) -> Res {
        let ids = self.inputs(input)?;
        for &id in &ids {
            if !self.d.set_stream_muted(id, muted)? {
                return Err(format!("could not change the mute of input {id}").into());
            }
        }
        self.say(format!("{} {} input(s)", if muted { "muted" } else { "unmuted" }, ids.len()));
        Ok(())
    }

    // ------------------------------------------------------------- fader

    fn fader_cmd(&self, cmd: Fader) -> Res {
        let d = &self.d;
        match cmd {
            Fader::Ls => {
                let master = d.get_master_gain()?;
                let pans: HashMap<u32, (f32, bool)> = d.list_group_pans()?.into_iter().map(|p| (p.0, (p.1, p.2))).collect();
                let mut rows = vec![json!({"id": null, "name": "master", "gain": n(master), "db": db(master), "pan": null, "mono": null})];
                rows.extend(d.list_groups()?.iter().map(|g| {
                    let (pan, mono) = pans.get(&g.0).copied().unwrap_or((0.0, false));
                    json!({"id": g.0, "name": g.1, "gain": n(g.2), "db": db(g.2), "pan": n(pan), "mono": mono})
                }));
                self.show(json!(rows));
            }
            Fader::Set { target, level } => {
                let groups = d.list_groups()?;
                let name = if target.eq_ignore_ascii_case("master") {
                    d.set_master_gain(level)?;
                    "master".to_owned()
                } else {
                    let id = self.group_in(&groups, &target)?;
                    d.set_gain(id, level)?;
                    name_of(&groups, id)
                };
                let at = if level > 0.0 { format!("{:+.1} dB", gain_db(level)) } else { "silence".to_owned() };
                self.say(format!("{name} at {at}"));
            }
            Fader::Pan { group, pan, mono, stereo } => {
                let id = self.group(&group)?;
                let current = d.list_group_pans()?.into_iter().find(|p| p.0 == id).unwrap_or((id, 0.0, false));
                if pan.is_none() && !mono && !stereo {
                    self.show(json!({"pan": n(current.1), "mono": current.2}));
                    return Ok(());
                }
                let pan = pan.unwrap_or(current.1);
                if !(-1.0..=1.0).contains(&pan) {
                    return Err("pan goes from -1 (left) to 1 (right)".into());
                }
                let mono = (current.2 || mono) && !stereo;
                d.set_pan(id, pan, mono)?;
                self.say(format!("pan {pan}, {}", if mono { "mono" } else { "stereo" }));
            }
            Fader::Levels { watch } => self.watch(watch, || {
                let groups = d.list_groups()?;
                let rows: Vec<Value> = d.get_levels()?.iter().map(|l| json!({"id": l.0, "name": name_of(&groups, l.0), "left_db": db(l.1), "right_db": db(l.2), "comp_gr_db": n(l.3), "gate_gr_db": n(l.4)})).collect();
                Ok(json!(rows))
            })?,
        }
        Ok(())
    }

    // --------------------------------------------------------------- dsp

    fn dsp_cmd(&self, cmd: Dsp) -> Res {
        let d = &self.d;
        match cmd {
            Dsp::Eq { group, band, freq, gain, q, toggle } => {
                let id = self.group(&group)?;
                let bands = d.get_group_details(id)?.0;
                let changing = freq.is_some() || gain.is_some() || q.is_some() || toggle.given();
                let Some(band) = band else {
                    if changing {
                        return Err("which band? (fonyx dsp eq <group> <band> ...)".into());
                    }
                    self.show(json!(eq_rows(&bands)));
                    return Ok(());
                };
                let Some(cur) = band.checked_sub(1).and_then(|i| bands.get(i as usize)) else {
                    return Err(format!("no band {band}: this group has bands 1 to {}", bands.len()).into());
                };
                if changing {
                    positive("--freq", freq)?;
                    positive("--q", q)?;
                    d.set_eq_band(id, band - 1, freq.unwrap_or(cur.0), gain.unwrap_or(cur.1), q.unwrap_or(cur.2), toggle.apply(cur.3))?;
                }
                let bands = d.get_group_details(id)?.0;
                self.show(json!([eq_rows(&bands)[band as usize - 1]]));
            }
            Dsp::EqLevel { group, db } => {
                let id = self.group(&group)?;
                if let Some(db) = db {
                    d.set_eq_level(id, db)?;
                }
                self.show(json!({"eq_level_db": n(d.get_eq_level(id)?)}));
            }
            Dsp::Hpf { group, filter } => self.filter_cmd(&group, filter, true)?,
            Dsp::Lpf { group, filter } => self.filter_cmd(&group, filter, false)?,
            Dsp::Comp { group, threshold, ratio, attack, release, makeup, toggle } => {
                let id = self.group(&group)?;
                let cur = d.get_group_details(id)?.3;
                if threshold.is_some() || ratio.is_some() || attack.is_some() || release.is_some() || makeup.is_some() || toggle.given() {
                    if ratio.is_some_and(|r| r < 1.0) {
                        return Err("--ratio must be 1 or more".into());
                    }
                    positive("--attack", attack)?;
                    positive("--release", release)?;
                    d.set_compressor(id, threshold.unwrap_or(cur.0), ratio.unwrap_or(cur.1), attack.unwrap_or(cur.2), release.unwrap_or(cur.3), makeup.unwrap_or(cur.4), toggle.apply(cur.5))?;
                }
                self.show(comp_obj(d.get_group_details(id)?.3));
            }
            Dsp::Gate { group, threshold, attack, release, hold, toggle } => {
                let id = self.group(&group)?;
                let cur = d.get_group_details(id)?.4;
                if threshold.is_some() || attack.is_some() || release.is_some() || hold.is_some() || toggle.given() {
                    positive("--attack", attack)?;
                    positive("--release", release)?;
                    if hold.is_some_and(|h| h < 0.0) {
                        return Err("--hold cannot be negative".into());
                    }
                    d.set_gate(id, threshold.unwrap_or(cur.0), attack.unwrap_or(cur.1), release.unwrap_or(cur.2), hold.unwrap_or(cur.3), toggle.apply(cur.4))?;
                }
                self.show(gate_obj(d.get_group_details(id)?.4));
            }
            Dsp::Plugin(cmd) => self.plugin_cmd(cmd)?,
        }
        Ok(())
    }

    fn filter_cmd(&self, group: &str, f: FilterArgs, highpass: bool) -> Res {
        let d = &self.d;
        let id = self.group(group)?;
        let read = || -> Res<(f32, f32, bool)> {
            let details = d.get_group_details(id)?;
            Ok(if highpass { details.1 } else { details.2 })
        };
        if f.freq.is_some() || f.q.is_some() || f.toggle.given() {
            positive("--freq", f.freq)?;
            positive("--q", f.q)?;
            let cur = read()?;
            let (freq, q, enabled) = (f.freq.unwrap_or(cur.0), f.q.unwrap_or(cur.1), f.toggle.apply(cur.2));
            if highpass {
                d.set_highpass(id, freq, q, enabled)?;
            } else {
                d.set_lowpass(id, freq, q, enabled)?;
            }
        }
        self.show(filter_obj(read()?));
        Ok(())
    }

    fn plugin_rows(&self, group_id: u32) -> Res<Vec<Value>> {
        Ok(self
            .d
            .list_group_plugins(group_id)?
            .into_iter()
            .map(|(index, kind, enabled, names, values)| {
                let params: serde_json::Map<String, Value> = names.into_iter().zip(values.into_iter().map(n)).collect();
                json!({"index": index, "plugin": kind, "enabled": enabled, "params": params})
            })
            .collect())
    }

    fn plugin_cmd(&self, cmd: Plugin) -> Res {
        let d = &self.d;
        // A slot the daemon doesn't have is ignored there without a word.
        let slot = |group: &str, index: u32| -> Res<(u32, PluginSlot)> {
            let id = self.group(group)?;
            let found = d.list_group_plugins(id)?.into_iter().find(|p| p.0 == index);
            Ok((id, found.ok_or_else(|| format!("no plugin slot {index} in that group (see: fonyx dsp plugin ls {group})"))?))
        };
        match cmd {
            Plugin::Ls { group } => self.show(json!(self.plugin_rows(self.group(&group)?)?)),
            Plugin::Available => {
                let rows: Vec<Value> = d
                    .list_available_plugins()?
                    .into_iter()
                    .map(|(kind, name, params)| {
                        let params: Vec<String> = params.iter().map(|(p, min, max, default)| format!("{p} {min}..{max} (default {default})")).collect();
                        json!({"plugin": kind, "name": name, "params": params})
                    })
                    .collect();
                self.show(json!(rows));
            }
            Plugin::Add { group, plugin } => {
                let id = self.group(&group)?;
                let known: Vec<String> = d.list_available_plugins()?.into_iter().map(|p| p.0).collect();
                if !known.contains(&plugin) {
                    return Err(format!("no plugin type \"{plugin}\". There are: {}", known.join(", ")).into());
                }
                d.add_group_plugin(id, &plugin)?;
                self.say(format!("added {plugin}"));
            }
            Plugin::Rm { group, index } => {
                let (id, (_, kind, ..)) = slot(&group, index)?;
                d.remove_group_plugin(id, index)?;
                self.say(format!("removed {kind}"));
            }
            Plugin::Bypass { group, index, off } => {
                let (id, (_, kind, ..)) = slot(&group, index)?;
                d.set_group_plugin_bypass(id, index, !off)?;
                self.say(format!("{kind} {}", if off { "back in the chain" } else { "bypassed" }));
            }
            Plugin::Set { group, index, param, value } => {
                let (id, (_, kind, _, names, _)) = slot(&group, index)?;
                if !names.contains(&param) {
                    return Err(format!("{kind} has no parameter \"{param}\". It has: {}", names.join(", ")).into());
                }
                d.set_group_plugin_param(id, index, &param, value)?;
                self.say(format!("{kind} {param} = {value}"));
            }
            Plugin::Move { group, from, to } => {
                let (id, (_, kind, ..)) = slot(&group, from)?;
                slot(&group, to)?;
                d.reorder_group_plugin(id, from, to)?;
                self.say(format!("moved {kind} to slot {to}"));
            }
        }
        Ok(())
    }

    // --------------------------------------------------------------- mtx

    /// Route tuples as rows; `bus` adds a leading column naming whose they are.
    fn route_rows(&self, bus: Option<&str>, routes: Vec<RouteRow>) -> Res<Vec<Value>> {
        let names = self.output_names()?;
        Ok(routes
            .into_iter()
            .map(|(id, gain, enabled, mode, sync, delay_ms, synced)| {
                let mut row = serde_json::Map::new();
                if let Some(bus) = bus {
                    row.insert("bus".into(), json!(bus));
                }
                let rest = json!({"output": names.get(&id).unwrap_or(&id), "gain": n(gain), "db": db(gain), "enabled": enabled, "mode": mode, "sync": sync, "delay_ms": n(delay_ms), "synced": synced, "id": id});
                row.extend(rest.as_object().unwrap().clone());
                Value::Object(row)
            })
            .collect())
    }

    fn routes_of(&self, bus: Option<u32>) -> Res<Vec<RouteRow>> {
        Ok(match bus {
            Some(id) => self.d.list_group_output_routes(id)?,
            None => self.d.list_master_output_routes()?,
        })
    }

    /// The bus to edit. The Cue group's routes belong to the cue outputs.
    fn editable_bus(&self, target: &str) -> Res<Option<u32>> {
        let bus = self.bus(target)?;
        if bus == Some(self.d.get_cue_group_id()?) {
            return Err("the Cue group plays on the cue outputs: use fonyx output cue add|rm".into());
        }
        Ok(bus)
    }

    fn mtx_cmd(&self, cmd: Mtx) -> Res {
        let d = &self.d;
        match cmd {
            Mtx::Ls { target: Some(target) } => {
                let bus = self.bus(&target)?;
                self.show(json!(self.route_rows(None, self.routes_of(bus)?)?));
            }
            Mtx::Ls { target: None } => {
                let mut rows = self.route_rows(Some("master"), self.routes_of(None)?)?;
                for g in d.list_groups()? {
                    rows.extend(self.route_rows(Some(&g.1), self.routes_of(Some(g.0))?)?);
                }
                self.show(json!(rows));
            }
            Mtx::Set { target, output, gain, toggle } => {
                let (bus, output) = (self.editable_bus(&target)?, self.output(&output)?);
                let current = self.routes_of(bus)?.into_iter().find(|r| r.0 == output);
                let (cur_gain, cur_enabled) = current.map_or((1.0, true), |r| (r.1, r.2));
                let (gain, enabled) = (gain.unwrap_or(cur_gain), toggle.apply(cur_enabled));
                match bus {
                    Some(id) => d.set_group_output_route(id, &output, gain, enabled)?,
                    None => d.set_master_output_route(&output, gain, enabled)?,
                }
                self.say(format!("route {}, {}", if enabled { "on" } else { "off" }, if gain > 0.0 { format!("{:+.1} dB", gain_db(gain)) } else { "silent".to_owned() }));
            }
            Mtx::Rm { target, output } => {
                let (bus, output) = (self.editable_bus(&target)?, self.output(&output)?);
                self.route(bus, &output)?;
                match bus {
                    Some(id) => d.remove_group_output_route(id, &output)?,
                    None => d.remove_master_output_route(&output)?,
                }
                self.say("route removed");
            }
            Mtx::Sync { target, output, mode, delay_ms } => {
                let (bus, output) = (self.editable_bus(&target)?, self.output(&output)?);
                let delay = delay_ms.unwrap_or(self.route(bus, &output)?.5);
                if delay < 0.0 {
                    return Err("a delay cannot be negative".into());
                }
                let mode = match mode {
                    SyncMode::Off => "off",
                    SyncMode::Automatic => "automatic",
                    SyncMode::Manual => "manual",
                };
                match bus {
                    Some(id) => d.set_output_synchronization(id, &output, mode, delay)?,
                    None => d.set_master_output_synchronization(&output, mode, delay)?,
                }
                self.say(format!("sync {mode}"));
            }
        }
        Ok(())
    }

    /// An existing route, or the error that says how to make one.
    fn route(&self, bus: Option<u32>, output: &str) -> Res<RouteRow> {
        Ok(self.routes_of(bus)?.into_iter().find(|r| r.0 == output).ok_or("there is no such route (add one with: fonyx mtx set <group|master> <output>)")?)
    }

    // ------------------------------------------------------------ output

    fn output_cmd(&self, cmd: Output) -> Res {
        let d = &self.d;
        let done = |ok: bool| -> Res { if ok { Ok(()) } else { Err("the device did not accept that (is it connected?)".into()) } };
        match cmd {
            Output::Ls => {
                let cue = d.list_cue_outputs()?;
                let mut rows = Vec::new();
                for (id, _, description, kind, available, default, rate, channels) in d.list_outputs()? {
                    // The volume is the device's own, so only a present one has it.
                    let volume = if available { Some(d.get_output_volume(&id)?) } else { None };
                    rows.push(json!({
                        "name": description, "kind": kind, "available": available, "volume": volume.map(|v| n(v.0)), "muted": volume.map(|v| v.1),
                        "default": default, "cue": cue.contains(&id), "rate": rate, "channels": channels, "id": id,
                    }));
                }
                self.show(json!(rows));
            }
            Output::Volume { output, level } => {
                let id = self.output(&output)?;
                if let Some(level) = level {
                    done(d.set_output_volume(&id, level)?)?;
                }
                let (volume, muted) = d.get_output_volume(&id)?;
                self.show(json!({"volume": n(volume), "muted": muted}));
            }
            Output::Mute { output } => {
                done(d.set_output_muted(&self.output(&output)?, true)?)?;
                self.say("muted");
            }
            Output::Unmute { output } => {
                done(d.set_output_muted(&self.output(&output)?, false)?)?;
                self.say("unmuted");
            }
            Output::Default { output: None } => {
                let id = d.get_preferred_default_output()?;
                let name = self.output_names()?.get(&id).cloned();
                self.show(json!({"default": name, "id": (!id.is_empty()).then_some(id)}));
            }
            Output::Default { output: Some(output) } => {
                if output.eq_ignore_ascii_case("none") {
                    d.set_preferred_default_output("")?;
                    self.say("no preferred output: unrouted groups follow the system default");
                } else {
                    d.set_preferred_default_output(&self.output(&output)?)?;
                    self.say("unrouted groups now play there");
                }
            }
            Output::Cue(Cue::Ls) => {
                let names = self.output_names()?;
                let rows: Vec<Value> = d.list_cue_outputs()?.into_iter().map(|id| json!({"name": names.get(&id).unwrap_or(&id), "id": id})).collect();
                self.show(json!(rows));
            }
            Output::Cue(Cue::Add { output }) => {
                d.set_cue_output(&self.output(&output)?, true)?;
                self.say("cue output added");
            }
            Output::Cue(Cue::Rm { output }) => {
                d.set_cue_output(&self.output(&output)?, false)?;
                self.say("cue output removed");
            }
            Output::Cue(Cue::Duck { level }) => {
                if let Some(level) = level {
                    d.set_cue_duck_level(level)?;
                }
                self.show(json!({"cue_duck_level": n(d.get_cue_duck_level()?)}));
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------- alias

    fn alias_cmd(&self, cmd: AliasCmd) -> Res {
        let mut aliases = Aliases(self.aliases.0.clone());
        match cmd {
            AliasCmd::Ls => {
                let rows: Vec<Value> = aliases.0.iter().map(|(name, a)| json!({"alias": name, "kind": a.kind, "target": a.target})).collect();
                self.show(json!(rows));
            }
            AliasCmd::Set { alias, target, kind } => {
                // An alias that reads as an id or a keyword could never be told apart from one.
                if alias.is_empty() || alias.parse::<u32>().is_ok() || ["master", "none"].contains(&alias.to_lowercase().as_str()) {
                    return Err(format!("\"{alias}\" cannot be an alias (not a number, `master` or `none`)").into());
                }
                let mut found: Vec<(Kind, String)> = Vec::new();
                for k in [Kind::Output, Kind::Input, Kind::Group].into_iter().filter(|k| kind.is_none_or(|want| want == *k)) {
                    if let Some(stable) = self.alias_target(k, &target)? {
                        found.push((k, stable));
                    }
                }
                let (kind, stable) = match &found[..] {
                    [] => return Err(format!("\"{target}\" is not an input, output or group that exists right now").into()),
                    [one] => one.clone(),
                    many => {
                        let kinds: Vec<&str> = many.iter().map(|(k, _)| k.as_str()).collect();
                        return Err(format!("\"{target}\" could be an {}: say which with --kind", kinds.join(" or an ")).into());
                    }
                };
                aliases.0.insert(alias.clone(), resolve::Alias { kind: kind.as_str().to_owned(), target: stable.clone() });
                aliases.save()?;
                self.say(format!("{alias} = {} {stable}", kind.as_str()));
            }
            AliasCmd::Rm { alias } => {
                if aliases.0.remove(&alias).is_none() {
                    return Err(format!("no alias \"{alias}\"").into());
                }
                aliases.save()?;
                self.say(format!("removed alias {alias}"));
            }
        }
        Ok(())
    }

    /// What an alias should store for `target` if it names one `kind` of
    /// thing: the identity that outlives a restart (an input's app id, not
    /// its stream id). `None` when it names nothing of that kind.
    fn alias_target(&self, kind: Kind, target: &str) -> Res<Option<String>> {
        Ok(match kind {
            Kind::Output => self.output(target).ok(),
            Kind::Group => self.group(target).ok().map(|id| id.to_string()),
            Kind::Input => {
                let Ok(ids) = self.inputs(target) else { return Ok(None) };
                let mut apps: Vec<String> = self.d.list_streams()?.into_iter().filter(|s| ids.contains(&s.0)).map(|s| s.4).collect();
                apps.dedup();
                match &apps[..] {
                    [app] => Some(app.clone()),
                    _ => return Err(format!("\"{target}\" matches inputs of several apps; an alias names one app").into()),
                }
            }
        })
    }

    // ------------------------------------------------------------- scene

    fn scene_cmd(&self, cmd: Scene) -> Res {
        let d = &self.d;
        match cmd {
            Scene::Ls => {
                let rows: Vec<Value> = d.list_scenes()?.into_iter().map(|(id, name, description)| json!({"id": id, "name": name, "description": (!description.is_empty()).then_some(description)})).collect();
                self.show(json!(rows));
            }
            Scene::Save { name } => self.created(&format!("scene {name}"), d.capture_current_as_scene(&name)?),
            Scene::Update { scene } => {
                if !d.update_scene(self.scene(&scene)?)? {
                    return Err("could not update that scene".into());
                }
                self.say("scene updated from the current state");
            }
            Scene::Dup { scene, name } => {
                let id = d.duplicate_scene(self.scene(&scene)?, &name)?;
                if id == NONE {
                    return Err("could not copy that scene".into());
                }
                self.created(&format!("scene {name}"), id);
            }
            Scene::Rm { scene } => {
                d.delete_scene(self.scene(&scene)?)?;
                self.say("scene deleted");
            }
            Scene::Apply { scene } => {
                d.activate_scene(self.scene(&scene)?)?;
                self.say("scene applied");
            }
        }
        Ok(())
    }

    // -------------------------------------------------------------- rule

    fn rule_cmd(&self, cmd: Rule) -> Res {
        let d = &self.d;
        match cmd {
            Rule::Ls => {
                let rows: Vec<Value> = d
                    .list_rules()?
                    .into_iter()
                    .map(|(id, name, enabled, priority, cooldown_ms, trigger, actions)| json!({"id": id, "name": name, "enabled": enabled, "priority": priority, "cooldown_ms": cooldown_ms, "when": trigger, "then": actions}))
                    .collect();
                self.show(json!(rows));
            }
            Rule::Show { rule } => {
                // Always JSON: it is what `rule save` takes back.
                let rule: Value = serde_json::from_str(&d.get_rule(self.rule(&rule)?)?)?;
                println!("{}", serde_json::to_string_pretty(&rule)?);
            }
            Rule::Save { file } => {
                let text = if file == "-" { std::io::read_to_string(std::io::stdin())? } else { std::fs::read_to_string(&file).map_err(|e| format!("{file}: {e}"))? };
                self.created("rule", d.save_rule(&text)?);
            }
            Rule::Enable { rule } => {
                d.set_rule_enabled(self.rule(&rule)?, true)?;
                self.say("rule enabled");
            }
            Rule::Disable { rule } => {
                d.set_rule_enabled(self.rule(&rule)?, false)?;
                self.say("rule disabled");
            }
            Rule::Rm { rule } => {
                d.delete_rule(self.rule(&rule)?)?;
                self.say("rule deleted");
            }
            Rule::History => {
                let rows: Vec<Value> = d
                    .rule_history()?
                    .into_iter()
                    .map(|(time, rule, trigger, actions, ok, message)| json!({"ago": ago(time), "rule": rule, "when": trigger, "then": actions, "ok": ok, "message": (!message.is_empty()).then_some(message), "time": time}))
                    .collect();
                self.show(json!(rows));
            }
        }
        Ok(())
    }

    // -------------------------------------------------------------- duck

    fn duck_cmd(&self, cmd: Duck) -> Res {
        let d = &self.d;
        // Ducking rules have no names, and a rule the daemon lacks is ignored silently.
        let known = |id: u32| -> Res { if d.list_ducking_rules()?.iter().any(|r| r.0 == id) { Ok(()) } else { Err(format!("no ducking rule {id} (see: fonyx duck ls)").into()) } };
        match cmd {
            Duck::Ls => {
                let groups = d.list_groups()?;
                let rows: Vec<Value> = d
                    .list_ducking_rules()?
                    .into_iter()
                    .map(|(id, enabled, trigger, targets, reduction, attack, hold, release, threshold)| {
                        let targets: Vec<String> = targets.iter().map(|&t| name_of(&groups, t)).collect();
                        json!({"id": id, "enabled": enabled, "trigger": name_of(&groups, trigger), "targets": targets, "reduction_db": n(reduction), "attack_ms": n(attack), "hold_ms": n(hold), "release_ms": n(release), "threshold_db": n(threshold)})
                    })
                    .collect();
                self.show(json!(rows));
            }
            Duck::Create { trigger, targets, reduction, attack, hold, release, threshold } => {
                let groups = d.list_groups()?;
                let trigger = self.group_in(&groups, &trigger)?;
                let targets = targets.iter().map(|t| self.group_in(&groups, t)).collect::<Res<Vec<u32>>>()?;
                if targets.contains(&trigger) {
                    return Err("a group cannot duck itself".into());
                }
                self.created("ducking rule", d.create_ducking_rule(trigger, targets, reduction, attack, hold, release, threshold)?);
            }
            Duck::Enable { id } => {
                known(id)?;
                d.set_ducking_rule_enabled(id, true)?;
                self.say("ducking rule enabled");
            }
            Duck::Disable { id } => {
                known(id)?;
                d.set_ducking_rule_enabled(id, false)?;
                self.say("ducking rule disabled");
            }
            Duck::Rm { id } => {
                known(id)?;
                d.delete_ducking_rule(id)?;
                self.say("ducking rule deleted");
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------ record

    fn record_cmd(&self, cmd: Record) -> Res {
        let d = &self.d;
        match cmd {
            Record::Ls => {
                let groups = d.list_groups()?;
                let rows: Vec<Value> = d.list_recordings()?.into_iter().map(|(id, group, path, seconds, frames)| json!({"id": id, "group": name_of(&groups, group), "seconds": n(seconds), "frames": frames, "path": path})).collect();
                self.show(json!(rows));
            }
            Record::Start { group, path, format } => {
                let format = match format {
                    Some(Format::Wav16) => "wav16",
                    Some(Format::Wav32) => "wav32",
                    Some(Format::Flac) => "flac",
                    None if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("flac")) => "flac",
                    None => "wav16",
                };
                // The daemon writes the file, from its own working directory.
                let path = std::path::absolute(&path)?;
                let id = d.start_recording(self.group(&group)?, &path.to_string_lossy(), format)?;
                if id == NONE {
                    return Err(format!("the daemon could not start recording to {}", path.display()).into());
                }
                if self.json {
                    self.show(json!({"id": id, "path": path}));
                } else {
                    self.say(format!("recording {format} to {} (id {id}); stop it with: fonyx record stop {id}", path.display()));
                }
            }
            Record::Stop { id } => {
                if !d.list_recordings()?.iter().any(|r| r.0 == id) {
                    return Err(format!("no recording {id} (see: fonyx record ls)").into());
                }
                d.stop_recording(id)?;
                self.say("recording stopped");
            }
        }
        Ok(())
    }
}
