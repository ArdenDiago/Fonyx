//! Consolidated automation engine
//! (NewImprovenments/02-context-aware-auto-routing.md +
//! 07-audio-rules-engine.md): one `Rule { trigger, conditions, actions,
//! priority, cooldown, enabled }` model serves both specs — 02's
//! auto-routing behaviors (Discord → Headphones, USB headset connected →
//! move comm apps) are rule templates over this same trigger/condition/
//! action machinery, not a second system (consolidation decision locked in
//! during the Phase 0 planning pass, before either spec was implemented).
//!
//! Structured data only — no arbitrary command execution (07's explicit
//! security instruction). Evaluation lives on the daemon's PipeWire engine
//! thread (`pw.rs`'s `Engine`), which is what already owns every piece of
//! state an action might touch (groups, routes, streams, outputs) and the
//! `Command`/persistence plumbing to change it safely — reusing that instead
//! of inventing a second execution path is also most of what keeps this
//! loop-safe: every `Action` here ultimately calls the same methods a
//! manual D-Bus call would, and none of those methods ever themselves raise
//! one of this module's `Event`s (only real PipeWire registry activity
//! does), so a rule's own actions structurally cannot re-trigger it.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::identity::{AppId, OutputId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: u32,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Higher wins when two matching rules touch the same property in the
    /// same evaluation (07 §"Conflict Handling") — enforced simply by
    /// evaluating matches in ascending priority order, so the highest
    /// priority rule's actions apply last and stick.
    #[serde(default)]
    pub priority: i32,
    /// Minimum time between this rule firing again, milliseconds — the
    /// debounce spec 02 asks for ("one application startup does not cause
    /// multiple conflicting evaluations").
    #[serde(default)]
    pub cooldown_ms: u64,
    pub trigger: Trigger,
    #[serde(default)]
    pub conditions: Vec<Condition>,
    pub actions: Vec<Action>,
}

fn default_true() -> bool {
    true
}

/// What a rule listens for. `None`/absent filters mean "any" — e.g.
/// `AppStarted { app_id: None }` fires for every new app stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Trigger {
    /// `service` matches the site/service name extracted from a browser
    /// tab's title (e.g. "Udemy", "YouTube" — see `pw.rs::extract_service_name`)
    /// case-insensitively. It's a separate filter from `app_id` (which is
    /// always the *browser's* identity, identical for every tab) so a rule
    /// can single out one site without needing PipeWire's exact
    /// `appname:`/`appid:` string.
    AppStarted { app_id: Option<AppId>, #[serde(default)] service: Option<String> },
    AppStopped { app_id: Option<AppId> },
    DeviceConnected { output_id: Option<OutputId> },
    DeviceDisconnected { output_id: Option<OutputId> },
    SceneActivated { scene_id: Option<u32> },
    /// Wake from suspend, from logind's `PrepareForSleep(false)`
    /// (`main.rs::watch_resume`).
    SystemResume,
}

/// A live occurrence of something a `Trigger` might match — always carries
/// the concrete subject (which app, which output) so actions can act on
/// "whatever just happened" without the rule having to hardcode it twice.
#[derive(Debug, Clone)]
pub enum Event {
    /// `service` is often empty at the moment a stream first appears — a
    /// browser tab's title arrives asynchronously (see `pw.rs`'s
    /// `on_global` doc comment) — and gets refired with the resolved value
    /// once known, so a service-filtered rule still gets a chance to match.
    AppStarted { app_id: AppId, stream_id: u32, service: String },
    AppStopped { app_id: AppId },
    DeviceConnected { output_id: OutputId },
    DeviceDisconnected { output_id: OutputId },
    SceneActivated { scene_id: u32 },
    SystemResume,
}

impl Event {
    /// The stream that raised this event, if it has one (cooldown key).
    pub fn stream_id(&self) -> Option<u32> {
        match self {
            Event::AppStarted { stream_id, .. } => Some(*stream_id),
            _ => None,
        }
    }
}

impl Trigger {
    fn matches(&self, event: &Event) -> bool {
        match (self, event) {
            (Trigger::AppStarted { app_id: filter, service: service_filter }, Event::AppStarted { app_id, service, .. }) => {
                filter.as_ref().is_none_or(|f| f == app_id)
                    && service_filter.as_ref().is_none_or(|f| service.eq_ignore_ascii_case(f))
            }
            (Trigger::AppStopped { app_id: filter }, Event::AppStopped { app_id }) => {
                filter.as_ref().is_none_or(|f| f == app_id)
            }
            (Trigger::DeviceConnected { output_id: filter }, Event::DeviceConnected { output_id }) => {
                filter.as_ref().is_none_or(|f| f == output_id)
            }
            (Trigger::DeviceDisconnected { output_id: filter }, Event::DeviceDisconnected { output_id }) => {
                filter.as_ref().is_none_or(|f| f == output_id)
            }
            (Trigger::SceneActivated { scene_id: filter }, Event::SceneActivated { scene_id }) => {
                filter.is_none_or(|f| f == *scene_id)
            }
            (Trigger::SystemResume, Event::SystemResume) => true,
            _ => false,
        }
    }
}

/// Extra gating beyond the trigger itself — spec 02's `if device.available
/// ("headphones")`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Condition {
    OutputAvailable(OutputId),
    OutputUnavailable(OutputId),
}

/// What a matching rule does. Deliberately closed to MTX's own operations
/// (07 §"Security": "do not allow arbitrary command execution") — every
/// variant maps straight onto an existing, already-tested `Engine`/`Config`
/// operation instead of introducing a new mutation path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    /// Routes the triggering app (from an `AppStarted`/`AppStopped` event)
    /// into `group_id`. Only meaningful for app-shaped triggers.
    RouteTriggeringAppToGroup { group_id: u32 },
    RouteGroupToOutput { group_id: u32, output_id: OutputId, gain: f32 },
    SetGroupVolume { group_id: u32, gain: f32 },
    ActivateScene { scene_id: u32 },
    /// Applies the profile matching the triggering app (see `profiles.rs`).
    ApplyProfileForTriggeringApp,
}

/// One row of `07`'s execution-history log: `timestamp | rule | trigger |
/// actions | result`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleExecution {
    pub rule_id: u32,
    pub rule_name: String,
    pub trigger_summary: String,
    pub actions_summary: String,
    pub ok: bool,
    pub message: String,
}

/// Bounded execution history, newest last, as (unix seconds, entry). A
/// static rather than a `RuleRuntime` field because the D-Bus thread reads
/// it while `RuleRuntime` lives on the PipeWire thread.
static HISTORY: std::sync::Mutex<std::collections::VecDeque<(u64, RuleExecution)>> =
    std::sync::Mutex::new(std::collections::VecDeque::new());

pub fn history() -> Vec<(u64, RuleExecution)> {
    HISTORY.lock().unwrap().iter().cloned().collect()
}

/// Runtime (non-persisted) state: cooldown timestamps. Kept separate from `Rule` itself so `Rule` stays a
/// plain, freely-cloneable, `Instant`-free config value (`Config` is
/// serialized to TOML — `Instant` isn't serializable and shouldn't be).
#[derive(Default)]
pub struct RuleRuntime {
    /// Keyed by (rule id, triggering stream): Firefox opens two streams back
    /// to back, and a per-rule cooldown left the second one un-routed.
    last_fired: std::collections::HashMap<(u32, Option<u32>), Instant>,
    /// Defensive recursion guard (07 §"Safety": "must never create infinite
    /// action loops"). Structurally this engine already can't loop — no
    /// `Action` here raises an `Event` — but this costs nothing and catches
    /// the mistake outright if that invariant is ever broken by a future
    /// action type.
    evaluating: bool,
}

const MAX_HISTORY: usize = 100;

impl RuleRuntime {
    fn record(&mut self, entry: RuleExecution) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut history = HISTORY.lock().unwrap();
        history.push_back((now, entry));
        while history.len() > MAX_HISTORY {
            history.pop_front();
        }
    }

    fn is_cooling_down(&self, rule_id: u32, stream_id: Option<u32>, cooldown_ms: u64) -> bool {
        self.last_fired.get(&(rule_id, stream_id)).is_some_and(|t| t.elapsed() < Duration::from_millis(cooldown_ms))
    }
}

/// Picks which of `rules` actually fire for `event` — enabled, matching
/// trigger, all conditions satisfied, not in cooldown — sorted so the
/// *caller* applying them in this order gets "highest priority wins" for
/// free (last write sticks). Pure function: takes whatever it needs to
/// evaluate conditions as plain data so it's fully unit-testable without a
/// live `Engine`.
pub fn matching_rules<'a>(
    rules: &'a [Rule],
    event: &Event,
    runtime: &RuleRuntime,
    output_available: impl Fn(&OutputId) -> bool,
) -> Vec<&'a Rule> {
    let mut matches: Vec<&Rule> = rules
        .iter()
        .filter(|r| r.enabled)
        .filter(|r| r.trigger.matches(event))
        .filter(|r| !runtime.is_cooling_down(r.id, event.stream_id(), r.cooldown_ms))
        .filter(|r| {
            r.conditions.iter().all(|c| match c {
                Condition::OutputAvailable(id) => output_available(id),
                Condition::OutputUnavailable(id) => !output_available(id),
            })
        })
        .collect();
    matches.sort_by_key(|r| r.priority);
    matches
}

impl RuleRuntime {
    /// Marks `rule_id` as having just fired (starts its cooldown) and logs
    /// the outcome.
    pub fn mark_fired(&mut self, rule_id: u32, stream_id: Option<u32>, entry: RuleExecution) {
        self.last_fired.insert((rule_id, stream_id), Instant::now());
        self.record(entry);
    }

    pub fn begin_evaluation(&mut self) -> bool {
        if self.evaluating {
            tracing::error!("rule evaluation re-entered — refusing to avoid a possible action loop");
            return false;
        }
        self.evaluating = true;
        true
    }

    pub fn end_evaluation(&mut self) {
        self.evaluating = false;
    }
}

/// Structural checks for a rule arriving over D-Bus (the rule builder).
/// Group/scene/output existence is the caller's job — it has the config.
pub fn validate(rule: &Rule) -> Result<(), String> {
    if rule.name.trim().is_empty() {
        return Err("rule needs a name".to_owned());
    }
    if rule.actions.is_empty() {
        return Err("rule needs at least one action".to_owned());
    }
    let app_trigger = matches!(rule.trigger, Trigger::AppStarted { .. });
    for action in &rule.actions {
        match action {
            Action::RouteTriggeringAppToGroup { .. } | Action::ApplyProfileForTriggeringApp if !app_trigger => {
                return Err("\"route app\" and \"apply profile\" need an app-started trigger".to_owned());
            }
            Action::RouteGroupToOutput { gain, .. } | Action::SetGroupVolume { gain, .. } if !(0.0..=2.0).contains(gain) => {
                return Err("gain must be between 0 and 2".to_owned());
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn describe_trigger(t: &Trigger) -> String {
    match t {
        Trigger::AppStarted { app_id, service } => match (app_id.as_deref(), service.as_deref()) {
            (None, None) => "app started (any)".to_owned(),
            (Some(app_id), None) => format!("app started ({app_id})"),
            (None, Some(service)) => format!("app started (site: {service})"),
            (Some(app_id), Some(service)) => format!("app started ({app_id}, site: {service})"),
        },
        Trigger::AppStopped { app_id } => format!("app stopped ({})", app_id.as_deref().unwrap_or("any")),
        Trigger::DeviceConnected { output_id } => format!("device connected ({})", output_id.as_deref().unwrap_or("any")),
        Trigger::DeviceDisconnected { output_id } => {
            format!("device disconnected ({})", output_id.as_deref().unwrap_or("any"))
        }
        Trigger::SceneActivated { scene_id } => {
            format!("scene activated ({})", scene_id.map_or("any".to_owned(), |id| id.to_string()))
        }
        Trigger::SystemResume => "system resume".to_owned(),
    }
}

/// `groups` resolves a `group_id` to its display name (falls back to
/// `#{id}` for a since-deleted group) so a rule's description reads e.g.
/// "route app to group Classes" instead of the bare, ambiguous numeric id.
pub fn describe_actions(actions: &[Action], groups: &[crate::group::Group]) -> String {
    let group_name = |id: u32| groups.iter().find(|g| g.id == id).map_or_else(|| format!("#{id}"), |g| g.name.clone());
    actions
        .iter()
        .map(|a| match a {
            Action::RouteTriggeringAppToGroup { group_id } => format!("route app to group {}", group_name(*group_id)),
            Action::RouteGroupToOutput { group_id, output_id, gain } => {
                format!("route group {} to {output_id} @ {gain:.2}", group_name(*group_id))
            }
            Action::SetGroupVolume { group_id, gain } => format!("set group {} volume to {gain:.2}", group_name(*group_id)),
            Action::ActivateScene { scene_id } => format!("activate scene {scene_id}"),
            Action::ApplyProfileForTriggeringApp => "apply profile for triggering app".to_owned(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: u32, priority: i32, trigger: Trigger, actions: Vec<Action>) -> Rule {
        Rule { id, name: format!("rule{id}"), enabled: true, priority, cooldown_ms: 0, trigger, conditions: Vec::new(), actions }
    }

    #[test]
    fn matches_specific_app_trigger_and_ignores_others() {
        let rules = vec![rule(1, 0, Trigger::AppStarted { app_id: Some("appid:discord".into()), service: None }, vec![])];
        let event = Event::AppStarted { app_id: "appid:discord".into(), stream_id: 5, service: String::new() };
        let runtime = RuleRuntime::default();
        assert_eq!(matching_rules(&rules, &event, &runtime, |_| true).len(), 1);

        let other_event = Event::AppStarted { app_id: "appid:spotify".into(), stream_id: 6, service: String::new() };
        assert_eq!(matching_rules(&rules, &other_event, &runtime, |_| true).len(), 0);
    }

    #[test]
    fn service_filter_matches_case_insensitively_and_ignores_other_sites() {
        let rules = vec![rule(
            1,
            0,
            Trigger::AppStarted { app_id: Some("appname:Firefox".into()), service: Some("Udemy".into()) },
            vec![],
        )];
        let runtime = RuleRuntime::default();

        let udemy = Event::AppStarted { app_id: "appname:Firefox".into(), stream_id: 1, service: "udemy".into() };
        assert_eq!(matching_rules(&rules, &udemy, &runtime, |_| true).len(), 1);

        let youtube = Event::AppStarted { app_id: "appname:Firefox".into(), stream_id: 2, service: "YouTube".into() };
        assert_eq!(matching_rules(&rules, &youtube, &runtime, |_| true).len(), 0);
    }

    #[test]
    fn wildcard_trigger_matches_any_app() {
        let rules = vec![rule(1, 0, Trigger::AppStarted { app_id: None, service: None }, vec![])];
        let event = Event::AppStarted { app_id: "appid:anything".into(), stream_id: 1, service: String::new() };
        let runtime = RuleRuntime::default();
        assert_eq!(matching_rules(&rules, &event, &runtime, |_| true).len(), 1);
    }

    #[test]
    fn disabled_rule_never_matches() {
        let mut r = rule(1, 0, Trigger::AppStarted { app_id: None, service: None }, vec![]);
        r.enabled = false;
        let event = Event::AppStarted { app_id: "x".into(), stream_id: 1, service: String::new() };
        assert_eq!(matching_rules(&[r], &event, &RuleRuntime::default(), |_| true).len(), 0);
    }

    #[test]
    fn condition_gates_the_match() {
        let mut r = rule(1, 0, Trigger::AppStarted { app_id: None, service: None }, vec![]);
        r.conditions.push(Condition::OutputAvailable("hp".into()));
        let event = Event::AppStarted { app_id: "x".into(), stream_id: 1, service: String::new() };
        assert_eq!(matching_rules(&[r.clone()], &event, &RuleRuntime::default(), |_| false).len(), 0);
        assert_eq!(matching_rules(&[r], &event, &RuleRuntime::default(), |_| true).len(), 1);
    }

    #[test]
    fn cooldown_suppresses_repeat_firing() {
        let r = rule(1, 0, Trigger::AppStarted { app_id: None, service: None }, vec![]);
        let event = Event::AppStarted { app_id: "x".into(), stream_id: 1, service: String::new() };
        let mut runtime = RuleRuntime::default();
        assert_eq!(matching_rules(std::slice::from_ref(&r), &event, &runtime, |_| true).len(), 1);
        runtime.mark_fired(1, event.stream_id(), RuleExecution {
            rule_id: 1,
            rule_name: "rule1".into(),
            trigger_summary: String::new(),
            actions_summary: String::new(),
            ok: true,
            message: String::new(),
        });
        let r_with_cooldown = Rule { cooldown_ms: 60_000, ..r };
        assert_eq!(matching_rules(std::slice::from_ref(&r_with_cooldown), &event, &runtime, |_| true).len(), 0, "should still be cooling down");
        // Firefox's second back-to-back stream must not inherit the first's cooldown.
        let other_stream = Event::AppStarted { app_id: "x".into(), stream_id: 2, service: String::new() };
        assert_eq!(matching_rules(&[r_with_cooldown], &other_stream, &runtime, |_| true).len(), 1, "cooldown is per stream");
    }

    #[test]
    fn ascending_priority_order_lets_highest_apply_last() {
        let low = rule(1, 1, Trigger::AppStarted { app_id: None, service: None }, vec![]);
        let high = rule(2, 10, Trigger::AppStarted { app_id: None, service: None }, vec![]);
        let event = Event::AppStarted { app_id: "x".into(), stream_id: 1, service: String::new() };
        let rules = [high, low];
        let matched = matching_rules(&rules, &event, &RuleRuntime::default(), |_| true);
        assert_eq!(matched.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1, 2], "lowest priority first, highest last");
    }

    #[test]
    fn history_is_bounded() {
        let mut runtime = RuleRuntime::default();
        for i in 0..150 {
            runtime.mark_fired(i, None, RuleExecution {
                rule_id: i,
                rule_name: String::new(),
                trigger_summary: String::new(),
                actions_summary: String::new(),
                ok: true,
                message: String::new(),
            });
        }
        assert_eq!(history().len(), MAX_HISTORY);
    }

    #[test]
    fn validate_rejects_app_action_without_app_trigger() {
        let r = rule(1, 0, Trigger::SystemResume, vec![Action::ApplyProfileForTriggeringApp]);
        assert!(validate(&r).is_err());
        let r = rule(1, 0, Trigger::SystemResume, vec![Action::SetGroupVolume { group_id: 0, gain: 0.5 }]);
        assert!(validate(&r).is_ok());
        let r = rule(1, 0, Trigger::SystemResume, vec![Action::SetGroupVolume { group_id: 0, gain: 9.0 }]);
        assert!(validate(&r).is_err());
        assert!(validate(&rule(1, 0, Trigger::SystemResume, vec![])).is_err());
    }

    #[test]
    fn rule_json_round_trips() {
        // The UI builds this exact shape; keep it in sync with index.html's buildRuleFromForm().
        let json = r#"{"id":0,"name":"n","enabled":true,"priority":0,"cooldown_ms":2000,
            "trigger":{"AppStarted":{"app_id":null,"service":"Udemy"}},
            "conditions":[{"OutputAvailable":"serial:x"}],
            "actions":[{"RouteTriggeringAppToGroup":{"group_id":1}},"ApplyProfileForTriggeringApp"]}"#;
        let r: Rule = serde_json::from_str(json).unwrap();
        assert!(validate(&r).is_ok());
        let back: Rule = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back.actions.len(), 2);
    }
}
