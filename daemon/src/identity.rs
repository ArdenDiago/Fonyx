//! Stable identity for app streams and output devices
//! (NewImprovenments/05-crash-safe-audio-sessions.md,
//! MTX_AUDIO_GROUPS_MULTI_OUTPUT.md §12): what gets persisted instead of a
//! transient PipeWire node id, which can change across an app restart, a
//! device reconnect, or a PipeWire restart. Every id is a plain, prefix-
//! tagged `String` — inspectable straight in the TOML config, no wrapper
//! type needed (same "just a String" choice the rest of this codebase
//! already makes for group/stream names).

use std::collections::HashMap;

use pipewire as pw;
use pw::spa::utils::dict::DictRef;

pub type AppId = String;
pub type OutputId = String;

/// A source of PipeWire-style key/value properties. Implemented for a live
/// `&DictRef` (a Node's own properties, straight from a registry event) and
/// for an owned `HashMap<String, String>` (`pw.rs` caches a Device global's
/// properties this way, since its `DictRef` doesn't outlive the callback
/// that announced it, unlike the Node whose props are read in the same
/// callback that needs them) — output identity/kind need to join both
/// sources, so the functions below are generic over this instead of forcing
/// everything through one concrete dict type.
pub trait Props {
    fn get_prop(&self, key: &str) -> Option<&str>;
}

impl Props for DictRef {
    fn get_prop(&self, key: &str) -> Option<&str> {
        self.get(key)
    }
}

impl Props for HashMap<String, String> {
    fn get_prop(&self, key: &str) -> Option<&str> {
        self.get(key).map(String::as_str)
    }
}

/// Stable identity for an application audio stream: prefers the desktop
/// application id (stable across restarts, distinguishes e.g. Firefox from
/// Chromium), falls back to the free-text application name, then the raw
/// PipeWire node name — in that order, matching spec 03/05's own guidance
/// ("desktop application id, then PipeWire application name, then
/// executable/node name as fallback").
pub fn app_identity(props: &DictRef) -> AppId {
    if let Some(id) = props.get("application.id") {
        return format!("appid:{id}");
    }
    if let Some(name) = props.get("application.name") {
        return format!("appname:{name}");
    }
    let node_name = props.get("node.name").unwrap_or("unknown");
    format!("node:{node_name}")
}

/// Stable identity for an output device, joining a Sink/Duplex Node's own
/// properties with its parent Device global's properties (`device_props`,
/// looked up via the node's `device.id` — a Node global alone doesn't carry
/// `device.serial`/Bluetooth address, only its Device global does). Order
/// per MTX_AUDIO_GROUPS_MULTI_OUTPUT.md §12: device serial, then Bluetooth
/// address, then USB/PCI vendor+product+card, then a name-based fallback —
/// "prefer the most stable available identifier and gracefully fall back".
pub fn output_identity<A: Props, B: Props>(node_props: &A, device_props: Option<&B>) -> OutputId {
    if let Some(dev) = device_props {
        if let Some(serial) = dev.get_prop("device.serial") {
            return format!("serial:{serial}");
        }
        if let Some(addr) = dev.get_prop("api.bluez5.address") {
            return format!("bt:{addr}");
        }
        if let (Some(vendor), Some(product)) = (dev.get_prop("device.vendor.id"), dev.get_prop("device.product.id")) {
            // PCI devices carry vendor/product ids too, not just USB ones
            // (confirmed live: a machine's built-in PCI HDA codec has both)
            // — tag with the real bus rather than assuming "usb:", which
            // this branch used to hard-code regardless of actual bus type.
            let bus = dev.get_prop("device.bus").unwrap_or("dev");
            let card = dev.get_prop("alsa.card").unwrap_or("0");
            return format!("{bus}:{vendor}:{product}:{card}");
        }
    }
    // Some setups expose the bluez address straight on the node too.
    if let Some(addr) = node_props.get_prop("api.bluez5.address") {
        return format!("bt:{addr}");
    }
    let name = node_props
        .get_prop("node.name")
        .or_else(|| node_props.get_prop("node.description"))
        .unwrap_or("unknown-output");
    format!("name:{name}")
}

/// Coarse device type for the Outputs UI (MTX_AUDIO_GROUPS_MULTI_OUTPUT.md
/// §4) — inferred from PipeWire properties, never guessed from the display
/// name (that section's explicit instruction).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    Headphones,
    Speaker,
    Bluetooth,
    Usb,
    Hdmi,
    DisplayPort,
    Virtual,
    Other,
}

impl OutputKind {
    /// Matches MTX_AUDIO_GROUPS_MULTI_OUTPUT.md §4's listed device types
    /// verbatim — used for the D-Bus-facing string and logging.
    pub fn as_str(&self) -> &'static str {
        match self {
            OutputKind::Headphones => "headphones",
            OutputKind::Speaker => "speaker",
            OutputKind::Bluetooth => "bluetooth",
            OutputKind::Usb => "usb",
            OutputKind::Hdmi => "hdmi",
            OutputKind::DisplayPort => "displayport",
            OutputKind::Virtual => "virtual",
            OutputKind::Other => "other",
        }
    }
}

pub fn output_kind<A: Props, B: Props>(node_props: &A, device_props: Option<&B>) -> OutputKind {
    let get = |key: &str| node_props.get_prop(key).or_else(|| device_props.and_then(|d| d.get_prop(key)));
    if get("api.bluez5.address").is_some() || get("device.bus") == Some("bluetooth") {
        return OutputKind::Bluetooth;
    }
    match get("device.form-factor") {
        Some("headphone") | Some("headset") => return OutputKind::Headphones,
        Some("hdmi") => return OutputKind::Hdmi,
        Some("displayport") => return OutputKind::DisplayPort,
        Some("speaker") => return OutputKind::Speaker,
        _ => {}
    }
    match get("device.bus") {
        Some("usb") => OutputKind::Usb,
        Some("pci") => OutputKind::Speaker,
        _ if device_props.is_none() => OutputKind::Virtual,
        _ => OutputKind::Other,
    }
}

/// Best-effort output latency (MTX_MULTI_OUTPUT_DELAY_COMPENSATION.md §9):
/// parses the `node.latency` property some session managers set, in its
/// `"quantum/rate"` form (e.g. `"1024/48000"`) — a real number PipeWire
/// itself provides, not a guess. Anything else (missing, malformed, or a
/// backend — notably Bluetooth — that doesn't report it) is `Unknown`
/// rather than an invented value, per that section's explicit instruction
/// never to hard-code arbitrary latency numbers.
pub fn estimate_output_latency<P: Props>(props: &P) -> crate::output::OutputLatency {
    use crate::output::OutputLatency;
    if let Some(raw) = props.get_prop("node.latency") {
        if let Some((num, den)) = raw.split_once('/') {
            if let (Ok(quantum), Ok(rate)) = (num.parse::<f32>(), den.parse::<f32>()) {
                if rate > 0.0 {
                    return OutputLatency::Measured(quantum / rate * 1000.0);
                }
            }
        }
    }
    OutputLatency::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::OutputLatency;
    use pipewire::spa::static_dict;

    #[test]
    fn app_identity_prefers_application_id() {
        let d = static_dict! {
            "application.id" => "org.mozilla.firefox",
            "application.name" => "Firefox",
            "node.name" => "firefox",
        };
        assert_eq!(app_identity(&d), "appid:org.mozilla.firefox");
    }

    #[test]
    fn app_identity_falls_back_to_name_then_node() {
        let with_name = static_dict! { "application.name" => "Spotify" };
        assert_eq!(app_identity(&with_name), "appname:Spotify");

        let node_only = static_dict! { "node.name" => "some-node" };
        assert_eq!(app_identity(&node_only), "node:some-node");
    }

    #[test]
    fn output_identity_prefers_serial_over_everything() {
        let node = static_dict! { "node.name" => "alsa_output.usb-foo" };
        let dev = static_dict! {
            "device.serial" => "ABC123",
            "api.bluez5.address" => "AA:BB:CC:DD:EE:FF",
        };
        assert_eq!(output_identity(&*node, Some(&*dev)), "serial:ABC123");
    }

    #[test]
    fn output_identity_bluetooth_without_serial() {
        let node = static_dict! { "node.name" => "bluez_output.AA_BB" };
        let dev = static_dict! { "api.bluez5.address" => "AA:BB:CC:DD:EE:FF" };
        assert_eq!(output_identity(&*node, Some(&*dev)), "bt:AA:BB:CC:DD:EE:FF");
    }

    #[test]
    fn output_identity_vendor_product_tagged_with_real_bus() {
        let node = static_dict! { "node.name" => "alsa_output.usb-x" };
        let dev = static_dict! {
            "device.vendor.id" => "0x046d",
            "device.product.id" => "0x0a44",
            "alsa.card" => "2",
            "device.bus" => "usb",
        };
        assert_eq!(output_identity(&*node, Some(&*dev)), "usb:0x046d:0x0a44:2");

        // A PCI device carries vendor/product ids too — must not be
        // mislabeled "usb:" just because that branch also handles it.
        let pci_dev = static_dict! {
            "device.vendor.id" => "0x8086",
            "device.product.id" => "0x51ca",
            "device.bus" => "pci",
        };
        assert_eq!(output_identity(&*node, Some(&*pci_dev)), "pci:0x8086:0x51ca:0");
    }

    #[test]
    fn output_identity_falls_back_to_name() {
        let node = static_dict! { "node.name" => "fonyx.group.0.route.headphones" };
        assert_eq!(output_identity::<DictRef, DictRef>(&node, None), "name:fonyx.group.0.route.headphones");
    }

    #[test]
    fn output_kind_detects_bluetooth() {
        let node = static_dict! { "node.name" => "bluez_output.x" };
        let dev = static_dict! { "api.bluez5.address" => "AA:BB:CC:DD:EE:FF" };
        assert_eq!(output_kind(&*node, Some(&*dev)), OutputKind::Bluetooth);
    }

    #[test]
    fn output_kind_detects_usb() {
        let node = static_dict! { "node.name" => "x" };
        let dev = static_dict! { "device.bus" => "usb" };
        assert_eq!(output_kind(&*node, Some(&*dev)), OutputKind::Usb);
    }

    #[test]
    fn output_kind_virtual_when_no_device() {
        let node = static_dict! { "media.class" => "Audio/Sink" };
        assert_eq!(output_kind::<DictRef, DictRef>(&node, None), OutputKind::Virtual);
    }

    #[test]
    fn latency_parses_quantum_over_rate() {
        let node = static_dict! { "node.latency" => "1024/48000" };
        assert_eq!(estimate_output_latency(&*node), OutputLatency::Measured(1024.0 / 48000.0 * 1000.0));
    }

    #[test]
    fn latency_unknown_when_absent_or_malformed() {
        let absent = static_dict! { "node.name" => "x" };
        assert_eq!(estimate_output_latency(&*absent), OutputLatency::Unknown);

        let malformed = static_dict! { "node.latency" => "garbage" };
        assert_eq!(estimate_output_latency(&*malformed), OutputLatency::Unknown);
    }
}
