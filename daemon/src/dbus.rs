//! D-Bus interface (PROJECT_SPEC.md Section 8, build-order stage 4):
//! `org.mtxpanel.Daemon` on the session bus. Not called from `main` yet —
//! stage 4 comes after the gain stage (stage 3) proves parameter smoothing
//! works. Wiring this in means: hold the `Arc<Mutex<Config>>` here, and have
//! `SetGain`/`SetEQBand`/etc. push through to each group's `SmoothedGain`
//! (control-thread side, so a mutex here is fine — Section 6's no-lock rule
//! is about the audio callback only).

use std::sync::{Arc, Mutex};

use zbus::interface;

use crate::config::Config;
use crate::group::Group;

pub struct Daemon {
    state: Arc<Mutex<Config>>,
}

impl Daemon {
    pub fn new(state: Arc<Mutex<Config>>) -> Self {
        Self { state }
    }
}

#[interface(name = "org.mtxpanel.Daemon")]
impl Daemon {
    fn create_group(&mut self, name: String) -> u32 {
        let mut cfg = self.state.lock().unwrap();
        let id = cfg.groups.iter().map(|g| g.id).max().map_or(0, |max| max + 1);
        cfg.groups.push(Group::new(id, name));
        let _ = cfg.save();
        id
    }

    fn delete_group(&mut self, group_id: u32) {
        let mut cfg = self.state.lock().unwrap();
        cfg.groups.retain(|g| g.id != group_id);
        let _ = cfg.save();
    }

    fn route_stream_to_group(&mut self, stream_id: u32, group_id: u32) {
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            group.routed_streams.push(stream_id);
        }
        let _ = cfg.save();
        // TODO(stage 2): actually link the PipeWire nodes, not just record it.
    }

    fn set_gain(&mut self, group_id: u32, value: f32) {
        let mut cfg = self.state.lock().unwrap();
        if let Some(group) = cfg.groups.iter_mut().find(|g| g.id == group_id) {
            group.gain = value;
        }
        let _ = cfg.save();
        // TODO(stage 3): forward to the live SmoothedGain::set_target, not
        // just the persisted value.
    }

    fn list_groups(&self) -> Vec<(u32, String)> {
        let cfg = self.state.lock().unwrap();
        cfg.groups.iter().map(|g| (g.id, g.name.clone())).collect()
    }
}
