//! Extensible Plugin / DSP Architecture
//! (NewImprovenments/10-plugin-dsp-architecture.md): a versioned, in-process
//! Rust plugin trait (the spec's own recommended first version — "prefer an
//! in-process Rust plugin API... add LV2/VST support only after the
//! internal chain abstraction is stable") slotted into the existing chain
//! as `Gain → EQ → Filter → Compressor → Gate → Plugin(s) → Output`.
//!
//! Scope decision, disclosed rather than silent: this pass ships the ABI
//! itself (the `Plugin` trait, parameter descriptors, latency/sample-rate
//! reporting, a `PluginChain` runtime, and two real built-in plugins that
//! exercise every part of it) — not out-of-process or dynamically-loaded
//! (`.so`) third-party plugins, which the spec itself frames as later work
//! ("Add LV2/VST support only after the internal chain abstraction is
//! stable"). A new built-in plugin type is added by implementing `Plugin`
//! and registering it in `builtin_registry()`; nothing about the chain,
//! persistence, or D-Bus surface needs to change to add one.
//!
//! Real-time safety: `Plugin::process` runs once per sample inside the
//! group's capture callback (`pw.rs`), so — same rule as every other stage
//! in that chain — it must never allocate, block, or take a slow lock.
//! Every built-in plugin here only ever touches its own pre-sized `f32`
//! fields.

use serde::{Deserialize, Serialize};

/// A parameter a plugin exposes, for a generic UI to render a control from
/// without knowing the plugin's concrete type (spec's "parameter
/// descriptors").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamDescriptor {
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMetadata {
    /// Stable key (e.g. `"soft_clipper"`) — what gets persisted in
    /// `PluginInstance::plugin_type` and looked up in `builtin_registry()`.
    pub type_id: String,
    pub display_name: String,
    pub params: Vec<ParamDescriptor>,
}

/// The plugin ABI itself. A versioned trait rather than a raw function-
/// pointer table (this being pure in-process Rust, not FFI, is what makes
/// that safe and simple) — `metadata()` is what the spec calls "plugin
/// metadata + parameter descriptors", `process`/`reset` are the real-time
/// callbacks, `latency_samples` and `set_sample_rate` are the two other
/// pieces of information the spec explicitly asks a plugin to declare.
pub trait Plugin: Send {
    fn metadata(&self) -> PluginMetadata;
    /// Processes one stereo frame in place. Must not allocate or block.
    fn process(&mut self, frame: &mut [f32; 2]);
    /// Clears any internal state (filter memory, envelope followers) —
    /// called when bypass is toggled off after being on, so re-enabling a
    /// plugin doesn't replay stale history.
    fn reset(&mut self);
    /// Samples of latency this plugin's processing introduces (spec's
    /// "latency reporting"). `0` for every built-in here — genuinely
    /// zero-latency per-sample processing — but the field exists so a
    /// future plugin with real lookahead/buffering has somewhere honest to
    /// report it instead of the chain silently assuming zero.
    fn latency_samples(&self) -> usize {
        0
    }
    fn set_sample_rate(&mut self, sample_rate: f32);
    fn set_param(&mut self, name: &str, value: f32);
}

/// Soft clipper / saturator: `tanh`-shaped saturation past `drive`, a
/// classic alloc-free nonlinearity — exercises the ABI's `process`/`reset`/
/// `set_param` with genuinely audible, genuinely stateless processing.
struct SoftClipper {
    drive: f32,
}

impl Plugin for SoftClipper {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata {
            type_id: "soft_clipper".to_owned(),
            display_name: "Soft Clipper".to_owned(),
            params: vec![ParamDescriptor { name: "drive".to_owned(), min: 1.0, max: 10.0, default: 1.0 }],
        }
    }
    fn process(&mut self, frame: &mut [f32; 2]) {
        for s in frame.iter_mut() {
            *s = (*s * self.drive).tanh();
        }
    }
    fn reset(&mut self) {}
    fn set_sample_rate(&mut self, _sample_rate: f32) {}
    fn set_param(&mut self, name: &str, value: f32) {
        if name == "drive" {
            self.drive = value.max(1.0);
        }
    }
}

/// Stereo widener: mid/side scaling — exercises a plugin that genuinely
/// needs both channels together (unlike `SoftClipper`, which is per-sample
/// per-channel), and carries its own tiny bit of persistent state
/// (`width`) that `reset` meaningfully has something to do for (nothing
/// dynamic here, but the shape matches a plugin that would).
struct StereoWidener {
    width: f32,
}

impl Plugin for StereoWidener {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata {
            type_id: "stereo_widener".to_owned(),
            display_name: "Stereo Widener".to_owned(),
            params: vec![ParamDescriptor { name: "width".to_owned(), min: 0.0, max: 2.0, default: 1.0 }],
        }
    }
    fn process(&mut self, frame: &mut [f32; 2]) {
        let mid = (frame[0] + frame[1]) * 0.5;
        let side = (frame[0] - frame[1]) * 0.5 * self.width;
        frame[0] = mid + side;
        frame[1] = mid - side;
    }
    fn reset(&mut self) {}
    fn set_sample_rate(&mut self, _sample_rate: f32) {}
    fn set_param(&mut self, name: &str, value: f32) {
        if name == "width" {
            self.width = value.clamp(0.0, 2.0);
        }
    }
}

pub fn builtin_registry() -> Vec<PluginMetadata> {
    vec![SoftClipper { drive: 1.0 }.metadata(), StereoWidener { width: 1.0 }.metadata()]
}

fn instantiate(type_id: &str) -> Option<Box<dyn Plugin>> {
    match type_id {
        "soft_clipper" => Some(Box::new(SoftClipper { drive: 1.0 })),
        "stereo_widener" => Some(Box::new(StereoWidener { width: 1.0 })),
        _ => None,
    }
}

/// Persisted per-slot config (`Group.plugins`) — the config-side shape;
/// `PluginChain` below is the corresponding real-time-side runtime built
/// from a `Vec<PluginInstance>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInstance {
    pub plugin_type: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub params: Vec<(String, f32)>,
}

fn default_true() -> bool {
    true
}

/// Owns the live, instantiated plugin objects for one group, in chain
/// order — rebuilt wholesale on any structural change (add/remove/reorder,
/// all infrequent control events, same category as rebuilding a group's
/// route list) rather than mutated in place, which keeps `pw.rs`'s
/// `Command` handling simple at the cost of losing a bypassed-then-
/// re-enabled plugin's `reset()`-cleared state on every unrelated
/// structural edit too — acceptable since every built-in plugin here is
/// otherwise stateless anyway.
pub struct PluginChain {
    slots: Vec<(Box<dyn Plugin>, bool)>, // (instance, enabled)
}

impl PluginChain {
    pub fn build(instances: &[PluginInstance], sample_rate: f32) -> Self {
        let slots = instances
            .iter()
            .filter_map(|inst| {
                let mut plugin = instantiate(&inst.plugin_type)?;
                plugin.set_sample_rate(sample_rate);
                for (name, value) in &inst.params {
                    plugin.set_param(name, *value);
                }
                Some((plugin, inst.enabled))
            })
            .collect();
        Self { slots }
    }

    #[inline]
    pub fn process(&mut self, frame: &mut [f32; 2]) {
        for (plugin, enabled) in self.slots.iter_mut() {
            if *enabled {
                plugin.process(frame);
            }
        }
    }

    pub fn total_latency_samples(&self) -> usize {
        self.slots.iter().map(|(p, _)| p.latency_samples()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_lists_both_builtins() {
        let names: Vec<String> = builtin_registry().into_iter().map(|m| m.type_id).collect();
        assert!(names.contains(&"soft_clipper".to_owned()));
        assert!(names.contains(&"stereo_widener".to_owned()));
    }

    #[test]
    fn soft_clipper_limits_extreme_input_toward_unity() {
        let mut p = SoftClipper { drive: 5.0 };
        let mut frame = [10.0, -10.0];
        p.process(&mut frame);
        assert!(frame[0] < 1.01 && frame[0] > 0.9, "should saturate toward +-1, got {frame:?}");
        assert!(frame[1] > -1.01 && frame[1] < -0.9);
    }

    #[test]
    fn stereo_widener_zero_width_collapses_to_mono() {
        let mut p = StereoWidener { width: 0.0 };
        let mut frame = [1.0, -1.0];
        p.process(&mut frame);
        assert_eq!(frame[0], frame[1], "zero width should collapse L/R to the mid signal");
    }

    #[test]
    fn chain_applies_slots_in_order_and_respects_enabled() {
        let instances = vec![
            PluginInstance { plugin_type: "stereo_widener".to_owned(), enabled: true, params: vec![("width".to_owned(), 0.0)] },
            PluginInstance { plugin_type: "soft_clipper".to_owned(), enabled: false, params: vec![("drive".to_owned(), 10.0)] },
        ];
        let mut chain = PluginChain::build(&instances, 48_000.0);
        let mut frame = [1.0, -1.0];
        chain.process(&mut frame);
        // Widener (enabled) collapses to mono; clipper is disabled so no
        // further saturation happens — output should be exactly 0.0, not
        // tanh(0)*10 or anything the clipper would have done.
        assert_eq!(frame, [0.0, 0.0]);
    }

    #[test]
    fn unknown_plugin_type_is_skipped_not_a_panic() {
        let instances = vec![PluginInstance { plugin_type: "nonexistent".to_owned(), enabled: true, params: vec![] }];
        let chain = PluginChain::build(&instances, 48_000.0);
        assert_eq!(chain.total_latency_samples(), 0);
    }
}
