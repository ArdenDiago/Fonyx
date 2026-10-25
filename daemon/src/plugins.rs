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

use crate::dsp::biquad::{Biquad, EqBand};
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
/// metadata + parameter descriptors", `process` is the real-time callback.
/// No `reset`: the chain is rebuilt with fresh instances on every change
/// (see `PluginChain`). No latency reporting: every built-in is zero-
/// latency; add it when a plugin with lookahead exists.
pub trait Plugin: Send {
    fn metadata(&self) -> PluginMetadata;
    /// Processes one stereo frame in place. Must not allocate or block.
    fn process(&mut self, frame: &mut [f32; 2]);
    fn set_sample_rate(&mut self, sample_rate: f32);
    fn set_param(&mut self, name: &str, value: f32);
}

/// Soft clipper / saturator: `tanh`-shaped saturation past `drive`, a
/// classic alloc-free nonlinearity — exercises the ABI's `process`/
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
    fn set_sample_rate(&mut self, _sample_rate: f32) {}
    fn set_param(&mut self, name: &str, value: f32) {
        if name == "drive" {
            self.drive = value.max(1.0);
        }
    }
}

/// Stereo widener: mid/side scaling — exercises a plugin that genuinely
/// needs both channels together (unlike `SoftClipper`, which is per-sample
/// per-channel),.
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
    fn set_sample_rate(&mut self, _sample_rate: f32) {}
    fn set_param(&mut self, name: &str, value: f32) {
        if name == "width" {
            self.width = value.clamp(0.0, 2.0);
        }
    }
}

/// Low-shelf boost for sub/bass weight. Stereo-linked: one shelf per
/// channel, same settings.
struct DeepBass {
    amount_db: f32,
    freq_hz: f32,
    sample_rate: f32,
    filters: [Biquad; 2],
}

impl DeepBass {
    fn new() -> Self {
        Self { amount_db: 0.0, freq_hz: 80.0, sample_rate: 48_000.0, filters: [Biquad::default(); 2] }
    }
    fn update(&mut self) {
        for f in &mut self.filters {
            f.set_lowshelf(self.freq_hz, self.amount_db, self.sample_rate);
        }
    }
}

impl Plugin for DeepBass {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata {
            type_id: "deep_bass".to_owned(),
            display_name: "Deep Bass".to_owned(),
            params: vec![
                ParamDescriptor { name: "amount".to_owned(), min: 0.0, max: 12.0, default: 0.0 },
                ParamDescriptor { name: "freq".to_owned(), min: 40.0, max: 200.0, default: 80.0 },
            ],
        }
    }
    fn process(&mut self, frame: &mut [f32; 2]) {
        for (s, f) in frame.iter_mut().zip(&mut self.filters) {
            *s = f.process(*s);
        }
    }
    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.update();
    }
    fn set_param(&mut self, name: &str, value: f32) {
        match name {
            "amount" => self.amount_db = value.clamp(0.0, 12.0),
            "freq" => self.freq_hz = value.clamp(40.0, 200.0),
            _ => return,
        }
        self.update();
    }
}

/// Broad (Q 0.7) peak for pushing vocals/instruments forward or back.
struct Mids {
    amount_db: f32,
    freq_hz: f32,
    sample_rate: f32,
    filters: [Biquad; 2],
}

impl Mids {
    fn new() -> Self {
        Self { amount_db: 0.0, freq_hz: 1000.0, sample_rate: 48_000.0, filters: [Biquad::default(); 2] }
    }
    fn update(&mut self) {
        let band = EqBand { freq_hz: self.freq_hz, gain_db: self.amount_db, q: 0.7, enabled: true };
        for f in &mut self.filters {
            f.set_peaking(&band, self.sample_rate);
        }
    }
}

impl Plugin for Mids {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata {
            type_id: "mids".to_owned(),
            display_name: "Mids".to_owned(),
            params: vec![
                ParamDescriptor { name: "amount".to_owned(), min: -12.0, max: 12.0, default: 0.0 },
                ParamDescriptor { name: "freq".to_owned(), min: 300.0, max: 4000.0, default: 1000.0 },
            ],
        }
    }
    fn process(&mut self, frame: &mut [f32; 2]) {
        for (s, f) in frame.iter_mut().zip(&mut self.filters) {
            *s = f.process(*s);
        }
    }
    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.update();
    }
    fn set_param(&mut self, name: &str, value: f32) {
        match name {
            "amount" => self.amount_db = value.clamp(-12.0, 12.0),
            "freq" => self.freq_hz = value.clamp(300.0, 4000.0),
            _ => return,
        }
        self.update();
    }
}

pub fn builtin_registry() -> Vec<PluginMetadata> {
    ["soft_clipper", "stereo_widener", "deep_bass", "mids"].iter().filter_map(|t| instantiate(t)).map(|p| p.metadata()).collect()
}

fn instantiate(type_id: &str) -> Option<Box<dyn Plugin>> {
    match type_id {
        "soft_clipper" => Some(Box::new(SoftClipper { drive: 1.0 })),
        "stereo_widener" => Some(Box::new(StereoWidener { width: 1.0 })),
        "deep_bass" => Some(Box::new(DeepBass::new())),
        "mids" => Some(Box::new(Mids::new())),
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
/// `Command` handling simple and means a re-enabled plugin always starts
/// with clean state.
pub struct PluginChain {
    slots: Vec<(Box<dyn Plugin>, bool)>, // (instance, enabled)
    /// The known instances each slot was built from, in order —
    /// `adopt_params` compares their (plugin_type, enabled) to tell a param
    /// tweak from a structural change.
    built_from: Vec<PluginInstance>,
}

impl PluginChain {
    pub fn build(instances: &[PluginInstance], sample_rate: f32) -> Self {
        // Unknown types (a plugin removed since the config was saved) are
        // skipped, here and so in `built_from` too.
        let mut slots = Vec::new();
        let mut built_from = Vec::new();
        for inst in instances {
            let Some(mut plugin) = instantiate(&inst.plugin_type) else { continue };
            plugin.set_sample_rate(sample_rate);
            for (name, value) in &inst.params {
                plugin.set_param(name, *value);
            }
            slots.push((plugin, inst.enabled));
            built_from.push(inst.clone());
        }
        Self { slots, built_from }
    }

    /// Takes `fresh`'s param values when both chains have the same shape,
    /// keeping this chain's filter history (a rebuild per knob step would
    /// click). Returns false when `fresh` should replace this chain instead.
    /// Neither allocates nor frees: it runs under the lock the capture
    /// callback takes every buffer, so `fresh` is built before the lock.
    pub fn adopt_params(&mut self, fresh: &mut PluginChain) -> bool {
        let same_shape = self.built_from.len() == fresh.built_from.len()
            && self.built_from.iter().zip(&fresh.built_from).all(|(a, b)| a.plugin_type == b.plugin_type && a.enabled == b.enabled);
        if !same_shape {
            return false;
        }
        for ((plugin, _), inst) in self.slots.iter_mut().zip(&fresh.built_from) {
            for (name, value) in &inst.params {
                plugin.set_param(name, *value);
            }
        }
        std::mem::swap(&mut self.built_from, &mut fresh.built_from);
        true
    }

    #[inline]
    pub fn process(&mut self, frame: &mut [f32; 2]) {
        for (plugin, enabled) in self.slots.iter_mut() {
            if *enabled {
                plugin.process(frame);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_lists_all_builtins() {
        let names: Vec<String> = builtin_registry().into_iter().map(|m| m.type_id).collect();
        assert_eq!(names, ["soft_clipper", "stereo_widener", "deep_bass", "mids"]);
    }

    /// Steady-state gain of a sine at `freq` after the filter settles.
    fn gain_db_at(p: &mut dyn Plugin, freq: f32) -> f32 {
        let (mut peak_in, mut peak_out) = (0f32, 0f32);
        for n in 0..48_000 {
            let x = (2.0 * std::f32::consts::PI * freq * n as f32 / 48_000.0).sin();
            let mut frame = [x, x];
            p.process(&mut frame);
            if n > 24_000 {
                peak_in = peak_in.max(x.abs());
                peak_out = peak_out.max(frame[0].abs());
            }
        }
        20.0 * (peak_out / peak_in).log10()
    }

    #[test]
    fn deep_bass_boosts_lows_and_leaves_highs() {
        let mut p = DeepBass::new();
        p.set_sample_rate(48_000.0);
        p.set_param("amount", 9.0);
        assert!((gain_db_at(&mut p, 30.0) - 9.0).abs() < 1.0);
        assert!(gain_db_at(&mut p, 5_000.0).abs() < 0.5);
    }

    #[test]
    fn mids_peaks_at_its_freq_and_defaults_to_flat() {
        let mut p = Mids::new();
        p.set_sample_rate(48_000.0);
        assert!(gain_db_at(&mut p, 1_000.0).abs() < 0.1, "default must be transparent");
        p.set_param("amount", -6.0);
        assert!((gain_db_at(&mut p, 1_000.0) + 6.0).abs() < 0.5);
        assert!(gain_db_at(&mut p, 30.0).abs() < 1.0);
    }

    #[test]
    fn unknown_plugin_type_does_not_force_a_rebuild_on_param_change() {
        let mut insts = vec![
            PluginInstance { plugin_type: "gone_plugin".to_owned(), enabled: true, params: vec![] },
            PluginInstance { plugin_type: "mids".to_owned(), enabled: true, params: vec![] },
        ];
        let mut chain = PluginChain::build(&insts, 48_000.0);
        insts[1].params.push(("amount".to_owned(), 3.0));
        assert!(chain.adopt_params(&mut PluginChain::build(&insts, 48_000.0)));
    }

    #[test]
    fn param_change_updates_in_place_but_structure_change_does_not() {
        let mut insts = vec![PluginInstance { plugin_type: "mids".to_owned(), enabled: true, params: vec![] }];
        let mut chain = PluginChain::build(&insts, 48_000.0);
        insts[0].params.push(("amount".to_owned(), 3.0));
        assert!(chain.adopt_params(&mut PluginChain::build(&insts, 48_000.0)));
        assert_eq!(chain.built_from[0].params, insts[0].params, "next comparison sees the new params");
        insts[0].enabled = false;
        assert!(!chain.adopt_params(&mut PluginChain::build(&insts, 48_000.0)));
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
        assert!(chain.slots.is_empty());
    }
}
