//! `sonicforge system`: the services (through `systemctl --user`, the same
//! units packaging/install-user.sh installs), health and engine settings.

use std::process::Command;

use serde_json::{json, Value};

use crate::args::{Setting, System, Target};
use crate::out::n;
use crate::{Ctx, Res};

const DAEMON: &str = "sonicforge-daemon.service";
const UI: &str = "sonicforge-ui.service";
const QUICKSHELL: &str = "sonicforge-quickshell.service";

fn systemctl(args: &[&str]) -> Res {
    if Command::new("systemctl").arg("--user").args(args).status()?.success() {
        return Ok(());
    }
    Err(format!("systemctl --user {} failed (are the services installed? packaging/install-user.sh)", args.join(" ")).into())
}

/// `active`, `inactive`, `failed`, ... or `unknown` without a user systemd.
fn state(unit: &str) -> String {
    let out = Command::new("systemctl").args(["--user", "is-active", unit]).output();
    let state = out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
    if state.is_empty() { "unknown".to_owned() } else { state }
}

/// Clears what keeps the daemon's service from starting: a daemon started by
/// hand still holds the D-Bus name, and earlier crashes leave a start limit.
fn clear_the_way() {
    if state(DAEMON) != "active" {
        // -f, not -x: process names are cut to 15 characters, and this one is 17.
        let _ = Command::new("pkill").args(["-f", "(^|/)sonicforge-daemon$"]).status();
    }
    let _ = Command::new("systemctl").args(["--user", "reset-failed", DAEMON, UI, QUICKSHELL]).stderr(std::process::Stdio::null()).status();
}

/// Returns once the daemon answers, so `system restart && scene apply x` works.
fn wait_for_daemon(c: &Ctx) -> Res {
    for _ in 0..50 {
        if c.d.get_master_gain().is_ok() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err("the daemon started but is not answering (see: sonicforge system logs)".into())
}

pub fn run(c: &Ctx, cmd: System) -> Res {
    let backend = |t: Option<Target>| !matches!(t, Some(Target::Frontend));
    let frontend = |t: Option<Target>| !matches!(t, Some(Target::Backend));
    match cmd {
        System::Status => {
            let mut status = json!({"backend": state(DAEMON), "frontend": state(UI), "quickshell": state(QUICKSHELL)});
            let up = match c.d.get_metrics() {
                Ok((cpu, ram, buffer_frames, underruns)) => {
                    let more = json!({
                        "daemon": "answering", "health": c.d.run_health_checks()?.0, "cpu_percent": n(cpu), "ram_percent": n(ram),
                        "buffer_frames": buffer_frames, "underruns": underruns, "buffer_ms": n(c.d.get_buffer_ms()?),
                        "max_compensation_ms": n(c.d.get_max_compensation_ms()?), "groups": c.d.list_groups()?.len(),
                        "inputs": c.d.list_streams()?.len(), "outputs": c.d.list_outputs()?.len(),
                    });
                    status.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());
                    true
                }
                Err(_) => {
                    status["daemon"] = json!("not answering");
                    false
                }
            };
            c.show(status);
            if !up {
                std::process::exit(1);
            }
        }
        System::Start { target } => {
            if backend(target) {
                clear_the_way();
                systemctl(&["start", DAEMON])?;
                wait_for_daemon(c)?;
            }
            if frontend(target) {
                systemctl(&["start", UI])?;
            }
            c.say("started");
        }
        System::Stop { target } => {
            if frontend(target) {
                systemctl(&["stop", UI, QUICKSHELL])?;
            }
            if backend(target) {
                systemctl(&["stop", DAEMON])?;
            }
            c.say("stopped");
        }
        System::Restart { target } => {
            if backend(target) {
                clear_the_way();
                systemctl(&["restart", DAEMON])?;
                wait_for_daemon(c)?;
            }
            if frontend(target) {
                systemctl(&["restart", UI])?;
                // Only if someone turned the widget on.
                systemctl(&["try-restart", QUICKSHELL])?;
            }
            c.say("restarted");
        }
        System::Doctor => {
            let (score, checks) = c.d.run_health_checks()?;
            let checks: Vec<Value> = checks.into_iter().map(|(check, severity, message)| json!({"check": check, "severity": severity, "message": message})).collect();
            c.show(json!({"score": score, "checks": checks}));
            if score == "error" {
                std::process::exit(1);
            }
        }
        System::Graph => {
            let (nodes, edges) = c.d.get_graph()?;
            let label = |id: &str| nodes.iter().find(|n| n.0 == id).map_or(id, |n| n.1.as_str()).to_owned();
            let edges: Vec<Value> = edges.iter().map(|(from, to)| json!({"from": label(from), "to": label(to), "from_id": from, "to_id": to})).collect();
            let nodes: Vec<Value> = nodes.iter().map(|(id, label, kind, state)| json!({"label": label, "kind": kind, "state": state, "id": id})).collect();
            c.show(json!({"nodes": nodes, "edges": edges}));
        }
        System::Logs { follow, lines } => {
            let mut journal = Command::new("journalctl");
            journal.args(["--user", "--no-pager", "-u", DAEMON, "-n", &lines.to_string()]);
            if follow {
                journal.arg("-f");
            }
            if !journal.status()?.success() {
                return Err("journalctl failed".into());
            }
        }
        System::Set { key, value } => {
            // The daemon clamps both; read back what it kept.
            let kept = match key {
                Setting::BufferMs => {
                    c.d.set_buffer_ms(value)?;
                    json!({"buffer_ms": n(c.d.get_buffer_ms()?)})
                }
                Setting::MaxCompensationMs => {
                    c.d.set_max_compensation_ms(value)?;
                    json!({"max_compensation_ms": n(c.d.get_max_compensation_ms()?)})
                }
            };
            c.show(kept);
        }
    }
    Ok(())
}
