//! Visual PipeWire Signal-Flow Graph
//! (NewImprovenments/06-visual-signal-flow-graph.md): a computed *view* over
//! state the daemon already tracks (streams, groups, outputs), not a
//! separate graph model to keep in sync — `dbus.rs`'s `get_graph` builds
//! one of these fresh on every call from exactly the same `Config`/
//! `streams`/`outputs` every other tab already reads.
//!
//! Scope decision: DSP stages (EQ, compressor, gate) aren't modeled as
//! their own graph nodes — they aren't separately identifiable objects in
//! this architecture, just the group node's own always-applied chain. A
//! group node visually stands in for "this app, with its DSP applied".

use serde::{Deserialize, Serialize};

use crate::group::Group;
use crate::output::OutputDevice;
use crate::pw::AppStream;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    /// "app" | "group" | "output".
    pub kind: String,
    /// "running"/"idle" for apps, "available"/"unavailable" for outputs,
    /// "active" for groups.
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

pub fn build_graph(groups: &[Group], streams: &[AppStream], outputs: &[OutputDevice]) -> Graph {
    let mut graph = Graph::default();

    for stream in streams {
        graph.nodes.push(GraphNode {
            id: format!("app:{}", stream.id),
            label: stream.name.clone(),
            kind: "app".to_owned(),
            state: if stream.running { "running" } else { "idle" }.to_owned(),
        });
    }

    for group in groups {
        graph.nodes.push(GraphNode { id: format!("group:{}", group.id), label: group.name.clone(), kind: "group".to_owned(), state: "active".to_owned() });
        for &stream_id in &group.routed_streams {
            graph.edges.push(GraphEdge { from: format!("app:{stream_id}"), to: format!("group:{}", group.id) });
        }
    }

    for output in outputs {
        graph.nodes.push(GraphNode {
            id: format!("output:{}", output.id),
            label: output.description.clone(),
            kind: "output".to_owned(),
            state: if output.available() { "available" } else { "unavailable" }.to_owned(),
        });
    }

    // "No explicit routes" only means the group *prefers* the system
    // default sink (pw.rs's bare-AUTOCONNECT implicit route) — it does not
    // by itself mean a link exists. Only draw the edge once we know which
    // real output the session manager actually resolved that preference
    // to (`is_system_default`, live from PipeWire), and only while that
    // output is actually present. A "Default output" node with no real
    // sink behind it would assert a connection that doesn't exist (§13-15).
    let resolved_default = outputs.iter().find(|o| o.is_system_default && o.available());

    for group in groups {
        if group.output_routes.is_empty() {
            if let Some(default_output) = resolved_default {
                graph.edges.push(GraphEdge { from: format!("group:{}", group.id), to: format!("output:{}", default_output.id) });
            }
        } else {
            for route in &group.output_routes {
                if route.enabled {
                    graph.edges.push(GraphEdge { from: format!("group:{}", group.id), to: format!("output:{}", route.output_id) });
                }
            }
        }
    }

    graph
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::group::GroupOutputRoute;
    use crate::identity::OutputKind;
    use crate::output::OutputLatency;

    fn stream(id: u32, name: &str) -> AppStream {
        AppStream { id, name: name.to_owned(), service: String::new(), running: true, app_id: format!("appid:{name}"), peak: Default::default(), side: None }
    }

    fn output(id: &str) -> OutputDevice {
        OutputDevice { id: id.to_owned(), name: id.to_owned(), description: id.to_owned(), kind: OutputKind::Speaker, node_id: Some(1), is_system_default: false, sample_rate: 48_000, channels: 2, latency: OutputLatency::Unknown }
    }

    fn default_output(id: &str) -> OutputDevice {
        OutputDevice { is_system_default: true, ..output(id) }
    }

    #[test]
    fn builds_app_to_group_edges() {
        let mut g = Group::new(0, "Default");
        g.routed_streams = vec![5];
        let graph = build_graph(&[g], &[stream(5, "Firefox")], &[]);
        assert!(graph.edges.iter().any(|e| e.from == "app:5" && e.to == "group:0"));
    }

    #[test]
    fn empty_routes_produce_no_edge_when_no_default_is_resolved() {
        // No output is currently the resolved system default (e.g. nothing
        // plugged in yet) — must not assert a connection that doesn't exist.
        let g = Group::new(0, "Default");
        let graph = build_graph(&[g], &[], &[]);
        assert!(graph.edges.is_empty());
    }

    #[test]
    fn empty_routes_resolve_to_the_actual_default_sink() {
        let g = Group::new(0, "Default");
        let graph = build_graph(&[g], &[], &[default_output("speakers")]);
        assert!(graph.edges.iter().any(|e| e.from == "group:0" && e.to == "output:speakers"));
    }

    #[test]
    fn empty_routes_ignore_an_unavailable_default() {
        // is_system_default but currently disconnected (node_id: None) —
        // still no real link exists.
        let mut o = default_output("bt");
        o.node_id = None;
        let g = Group::new(0, "Default");
        let graph = build_graph(&[g], &[], &[o]);
        assert!(graph.edges.is_empty());
    }

    #[test]
    fn explicit_routes_produce_group_to_output_edges() {
        let mut g = Group::new(0, "Music");
        g.output_routes.push(GroupOutputRoute::new("hp".to_owned(), crate::output::RouteMode::Explicit));
        let graph = build_graph(&[g], &[], &[output("hp")]);
        assert!(graph.edges.iter().any(|e| e.from == "group:0" && e.to == "output:hp"));
        assert!(!graph.edges.iter().any(|e| e.to == "output:default"));
    }

    #[test]
    fn disabled_route_produces_no_edge() {
        let mut g = Group::new(0, "Music");
        let mut route = GroupOutputRoute::new("hp".to_owned(), crate::output::RouteMode::Explicit);
        route.enabled = false;
        g.output_routes.push(route);
        let graph = build_graph(&[g], &[], &[output("hp")]);
        assert!(!graph.edges.iter().any(|e| e.to == "output:hp"));
    }
}
