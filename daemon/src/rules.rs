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
    AppStarted { app_id: Option<AppId> },
    AppStopped { app_id: Option<AppId> },
    DeviceConnected { output_id: Option<OutputId> },
    DeviceDisconnected { output_id: Option<OutputId> },
    SceneActivated { scene_id: Option<u32> },
    /// Modeled now for forward compatibility (07's own "optional later"
    /// framing for time-based triggers applies in spirit here too); not
    /// wired to a real systemd-logind `PrepareForSleep` listener this pass
    /// — see rules.rs's module doc in the final report, not silently
    /// dropped.
    SystemResume,
}

/// A live occurrence of something a `Trigger` might match — always carries
/// the concrete subject (which app, which output) so actions can act on
/// "whatever just happened" without the rule having to hardcode it twice.
#[derive(Debug, Clone)]
pub enum Event {
    AppStarted { app_id: AppId, stream_id: u32 },
    AppStopped { app_id: AppId },
    DeviceConnected { output_id: OutputId },
    DeviceDisconnected { output_id: OutputId },
    SceneActivated { scene_id: u32 },
    SystemResume,
}

impl Trigger {
    fn matches(&self, event: &Event) -> bool {
        match (self, event) {
            (Trigger::AppStarted { app_id: filter }, Event::AppStarted { app_id, .. }) => {
                filter.as_ref().is_none_or(|f| f == app_id)
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

/// Runtime (non-persisted) state: cooldown timestamps and a bounded
/// execution history. Kept separate from `Rule` itself so `Rule` stays a
/// plain, freely-cloneable, `Instant`-free config value (`Config` is
/// serialized to TOML — `Instant` isn't serializable and shouldn't be).
#[derive(Default)]
pub struct RuleRuntime {
    last_fired: std::collections::HashMap<u32, Instant>,
    history: std::collections::VecDeque<RuleExecution>,
    /// Defensive recursion guard (07 §"Safety": "must never create infinite
    /// action loops"). Structurally this engine already can't loop — no
    /// `Action` here raises an `Event` — but this costs nothing and catches
    /// the mistake outright if that invariant is ever broken by a future
    /// action type.
    evaluating: bool,
}

const MAX_HISTORY: usize = 100;

impl RuleRuntime {
    pub fn history(&self) -> impl Iterator<Item = &RuleExecution> {
        self.history.iter()
    }

    fn record(&mut self, entry: RuleExecution) {
        self.history.push_back(entry);
        while self.history.len() > MAX_HISTORY {
            self.history.pop_front();
        }
    }

    fn is_cooling_down(&self, rule_id: u32, cooldown_ms: u64) -> bool {
        self.last_fired.get(&rule_id).is_some_and(|t| t.elapsed() < Duration::from_millis(cooldown_ms))
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
        .filter(|r| !runtime.is_cooling_down(r.id, r.cooldown_ms))
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
    pub fn mark_fired(&mut self, rule_id: u32, entry: RuleExecution) {
        self.last_fired.insert(rule_id, Instant::now());
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

pub fn describe_trigger(t: &Trigger) -> String {
    match t {
        Trigger::AppStarted { app_id } => format!("app started ({})", app_id.as_deref().unwrap_or("any")),
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

pub fn describe_actions(actions: &[Action]) -> String {
    actions
        .iter()
        .map(|a| match a {
            Action::RouteTriggeringAppToGroup { group_id } => format!("route app to group {group_id}"),
            Action::RouteGroupToOutput { group_id, output_id, gain } => {
                format!("route group {group_id} to {output_id} @ {gain:.2}")
            }
            Action::SetGroupVolume { group_id, gain } => format!("set group {group_id} volume to {gain:.2}"),
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
        let rules = vec![rule(1, 0, Trigger::AppStarted { app_id: Some("appid:discord".into()) }, vec![])];
        let event = Event::AppStarted { app_id: "appid:discord".into(), stream_id: 5 };
        let runtime = RuleRuntime::default();
        assert_eq!(matching_rules(&rules, &event, &runtime, |_| true).len(), 1);

        let other_event = Event::AppStarted { app_id: "appid:spotify".into(), stream_id: 6 };
        assert_eq!(matching_rules(&rules, &other_event, &runtime, |_| true).len(), 0);
    }

    #[test]
    fn wildcard_trigger_matches_any_app() {
        let rules = vec![rule(1, 0, Trigger::AppStarted { app_id: None }, vec![])];
        let event = Event::AppStarted { app_id: "appid:anything".into(), stream_id: 1 };
        let runtime = RuleRuntime::default();
        assert_eq!(matching_rules(&rules, &event, &runtime, |_| true).len(), 1);
    }

    #[test]
    fn disabled_rule_never_matches() {
        let mut r = rule(1, 0, Trigger::AppStarted { app_id: None }, vec![]);
        r.enabled = false;
        let event = Event::AppStarted { app_id: "x".into(), stream_id: 1 };
        assert_eq!(matching_rules(&[r], &event, &RuleRuntime::default(), |_| true).len(), 0);
    }

    #[test]
    fn condition_gates_the_match() {
        let mut r = rule(1, 0, Trigger::AppStarted { app_id: None }, vec![]);
        r.conditions.push(Condition::OutputAvailable("hp".into()));
        let event = Event::AppStarted { app_id: "x".into(), stream_id: 1 };
        assert_eq!(matching_rules(&[r.clone()], &event, &RuleRuntime::default(), |_| false).len(), 0);
        assert_eq!(matching_rules(&[r], &event, &RuleRuntime::default(), |_| true).len(), 1);
    }

    #[test]
    fn cooldown_suppresses_repeat_firing() {
        let r = rule(1, 0, Trigger::AppStarted { app_id: None }, vec![]);
        let event = Event::AppStarted { app_id: "x".into(), stream_id: 1 };
        let mut runtime = RuleRuntime::default();
        assert_eq!(matching_rules(&[r.clone()], &event, &runtime, |_| true).len(), 1);
        runtime.mark_fired(1, RuleExecution {
            rule_id: 1,
            rule_name: "rule1".into(),
            trigger_summary: String::new(),
            actions_summary: String::new(),
            ok: true,
            message: String::new(),
        });
        let r_with_cooldown = Rule { cooldown_ms: 60_000, ..r };
        assert_eq!(matching_rules(&[r_with_cooldown], &event, &runtime, |_| true).len(), 0, "should still be cooling down");
    }

    #[test]
    fn ascending_priority_order_lets_highest_apply_last() {
        let low = rule(1, 1, Trigger::AppStarted { app_id: None }, vec![]);
        let high = rule(2, 10, Trigger::AppStarted { app_id: None }, vec![]);
        let event = Event::AppStarted { app_id: "x".into(), stream_id: 1 };
        let rules = [high, low];
        let matched = matching_rules(&rules, &event, &RuleRuntime::default(), |_| true);
        assert_eq!(matched.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1, 2], "lowest priority first, highest last");
    }

    #[test]
    fn history_is_bounded() {
        let mut runtime = RuleRuntime::default();
        for i in 0..150 {
            runtime.mark_fired(i, RuleExecution {
                rule_id: i,
                rule_name: String::new(),
                trigger_summary: String::new(),
                actions_summary: String::new(),
                ok: true,
                message: String::new(),
            });
        }
        assert_eq!(runtime.history().count(), MAX_HISTORY);
    }
}
