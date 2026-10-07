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
    /// (app_id, group_id) — which app should land in which group. Only
    /// written for apps whose streams all shared one group at capture:
    /// used for streams that didn't exist then (the app restarted, a new
    /// tab), where there's no per-stream placement to go on.
    pub stream_assignments: Vec<(AppId, u32)>,
    /// (stream_id, app_id, group_id) — exact per-stream placement at
    /// capture. Applied first, to streams still live with the same id and
    /// app (PipeWire reuses ids, hence the app check). Without this, an app
    /// with tabs split across groups (Firefox in two groups) collapsed
    /// into whichever group its app-level entry named.
    #[serde(default)]
    pub stream_placements: Vec<(u32, AppId, u32)>,
}

/// One stream's exact place: (stream id, app id, group id).
type Placement = (u32, AppId, u32);
/// Where any stream of an app goes when it has no exact place: (app id, group id).
type AppFallback = (AppId, u32);

/// Splits routed streams `(stream_id, app_id, group_id)` into exact
/// placements plus app-level fallbacks — the latter only for apps whose
/// streams all sat in one group (a split app has no single right answer).
pub fn build_assignments(routed: &[Placement]) -> (Vec<Placement>, Vec<AppFallback>) {
    let mut per_app: Vec<(AppId, Option<u32>)> = Vec::new(); // None = split
    for (_, app, gid) in routed {
        match per_app.iter_mut().find(|(a, _)| a == app) {
            Some((_, g)) if *g != Some(*gid) => *g = None,
            Some(_) => {}
            None => per_app.push((app.clone(), Some(*gid))),
        }
    }
    let app_level = per_app.into_iter().filter_map(|(a, g)| g.map(|g| (a, g))).collect();
    (routed.to_vec(), app_level)
}

/// Which live streams `live` `(stream_id, app_id)` to move where, plus the
/// app-level entries with no live stream at all (to retry when the app
/// starts). A stream with an exact placement is never touched by its
/// app's fallback.
pub fn plan_stream_moves(scene: &Scene, live: &[(u32, AppId)]) -> (Vec<(u32, u32)>, Vec<AppFallback>) {
    let mut moves: Vec<(u32, u32)> = scene
        .stream_placements
        .iter()
        .filter(|(sid, app, _)| live.iter().any(|(l, la)| l == sid && la == app))
        .map(|(sid, _, gid)| (*sid, *gid))
        .collect();
    let mut pending = Vec::new();
    for (app, gid) in &scene.stream_assignments {
        // Scenes saved before per-stream placements existed can list one
        // app under several groups; there's no right answer, so skip it.
        if scene.stream_assignments.iter().any(|(a, g)| a == app && g != gid) {
            continue;
        }
        let of_app: Vec<u32> = live.iter().filter(|(_, a)| a == app).map(|(s, _)| *s).collect();
        if of_app.is_empty() {
            pending.push((app.clone(), *gid));
        }
        for sid in of_app {
            if !moves.iter().any(|(m, _)| *m == sid) {
                moves.push((sid, *gid));
            }
        }
    }
    (moves, pending)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_app_keeps_each_tab_in_its_own_group() {
        let ff = "appname:Firefox".to_owned();
        // Two tabs in group 5, one in group 6: the reported bug.
        let routed = vec![(75, ff.clone(), 5), (111, ff.clone(), 5), (102, ff.clone(), 6)];
        let (placements, app_level) = build_assignments(&routed);
        assert!(app_level.is_empty(), "split app must not get a collapsing app-level entry");
        let scene = Scene { id: 0, name: "s".into(), description: String::new(), master_gain: 1.0, groups: vec![], stream_assignments: app_level, stream_placements: placements };
        let live = vec![(75, ff.clone()), (111, ff.clone()), (102, ff.clone())];
        let (mut moves, pending) = plan_stream_moves(&scene, &live);
        moves.sort();
        assert_eq!(moves, vec![(75, 5), (102, 6), (111, 5)]);
        assert!(pending.is_empty());

        // An old-format scene (no placements, conflicting app entries)
        // leaves the split app's streams where they are.
        let old = Scene { stream_placements: vec![], stream_assignments: vec![(ff.clone(), 5), (ff.clone(), 6)], ..scene.clone() };
        assert!(plan_stream_moves(&old, &live).0.is_empty());

        // A reused id now belonging to another app is left alone.
        let (moves, _) = plan_stream_moves(&scene, &[(102, "appname:mpv".to_owned())]);
        assert!(moves.is_empty());
    }

    #[test]
    fn unsplit_app_still_follows_its_group_after_restart() {
        let sp = "appname:Spotify".to_owned();
        let (placements, app_level) = build_assignments(&[(40, sp.clone(), 3)]);
        assert_eq!(app_level, vec![(sp.clone(), 3)]);
        let scene = Scene { id: 0, name: "s".into(), description: String::new(), master_gain: 1.0, groups: vec![], stream_assignments: app_level, stream_placements: placements };
        // Restarted app: new stream id, falls back to the app-level entry.
        assert_eq!(plan_stream_moves(&scene, &[(90, sp.clone())]).0, vec![(90, 3)]);
        // Not running: pending for later.
        assert_eq!(plan_stream_moves(&scene, &[]).1, vec![(sp, 3)]);
    }
}
