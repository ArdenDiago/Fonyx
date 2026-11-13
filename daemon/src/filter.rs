//! One PipeWire `pw_filter` node with two DSP input ports and two DSP output
//! ports (FL, FR) that runs a closure every graph cycle.
//!
//! Why a filter and not a capture stream plus playback streams: a filter is
//! a single node, linked port to port, so PipeWire schedules it in the same
//! cycle as whatever feeds it and whatever it feeds, and mixes several links
//! into one input port by itself. The old design bridged two unlinked
//! streams through a ring buffer, which cost ~85 ms of latency, its own
//! class of underruns, and a hand-written mixing point for Master.
//!
//! pipewire-rs 0.10 has no safe wrapper for `pw_filter`, so this file is the
//! one place with raw FFI. Ports are DSP ports: mono f32 at the graph rate.

use std::ffi::{c_void, CString};
use std::ptr;

use pipewire as pw;
use pw::properties::{properties, PropertiesBox};
use pw::spa::sys as spa_sys;
use pw::sys as pw_sys;

/// Largest cycle handled; PipeWire's default `clock.max-quantum` is 8192.
/// A bigger cycle is processed only up to this many frames.
const MAX_FRAMES: usize = 8192;

/// `(inputs, outputs, sample_rate)`, called on PipeWire's realtime thread.
/// Same rules as any other `process` callback here: no allocation, no
/// locks held for long, no I/O.
pub type Process = Box<dyn FnMut([&[f32]; 2], [&mut [f32]; 2], f32) + Send>;

struct State {
    inputs: [*mut c_void; 2],
    outputs: [*mut c_void; 2],
    /// Stands in for an input with nothing linked (PipeWire hands back NULL).
    silence: Box<[f32]>,
    /// Stands in for an output PipeWire has no buffer for yet.
    scratch: [Box<[f32]>; 2],
    process: Process,
}

pub struct DspFilter {
    raw: *mut pw_sys::pw_filter,
    state: *mut State,
    // Both must outlive `raw`: `pw_filter_destroy` walks the hook list.
    _hook: Box<spa_sys::spa_hook>,
    _events: Box<pw_sys::pw_filter_events>,
}

unsafe extern "C" fn on_process(data: *mut c_void, position: *mut spa_sys::spa_io_position) {
    // SAFETY: `data` is the `State` registered in `DspFilter::new`, alive
    // until after `pw_filter_destroy` returns; only this thread touches it.
    let state = unsafe { &mut *(data as *mut State) };
    if position.is_null() {
        return;
    }
    let clock = unsafe { &(*position).clock };
    let n = (clock.duration as usize).min(MAX_FRAMES);
    let rate = if clock.rate.num > 0 { clock.rate.denom as f32 / clock.rate.num as f32 } else { 48_000.0 };
    let State { inputs, outputs, silence, scratch, process } = state;
    let input = |port: *mut c_void| -> &[f32] {
        let buf = unsafe { pw_sys::pw_filter_get_dsp_buffer(port, n as u32) } as *const f32;
        if buf.is_null() { &silence[..n] } else { unsafe { std::slice::from_raw_parts(buf, n) } }
    };
    let ins = [input(inputs[0]), input(inputs[1])];
    let [s0, s1] = scratch;
    let output = |port: *mut c_void, fallback: &'_ mut [f32]| -> *mut [f32] {
        let buf = unsafe { pw_sys::pw_filter_get_dsp_buffer(port, n as u32) } as *mut f32;
        if buf.is_null() { &mut fallback[..n] } else { ptr::slice_from_raw_parts_mut(buf, n) }
    };
    let (o0, o1) = (output(outputs[0], s0), output(outputs[1], s1));
    // SAFETY: distinct port buffers (or distinct scratch boxes), valid for
    // this cycle.
    process(ins, unsafe { [&mut *o0, &mut *o1] }, rate);
}

impl DspFilter {
    /// `node_props` must include `node.name`. `ports` names the input pair
    /// and output pair, FL first; each port also gets its `audio.channel`,
    /// which is what `Engine` pairs ports by when linking.
    pub fn new(core: &pw::core::CoreRc, node_props: PropertiesBox, ports: ([&str; 2], [&str; 2]), process: Process) -> Result<Self, String> {
        let name = CString::new(node_props.get("node.name").unwrap_or("fonyx.filter")).map_err(|e| e.to_string())?;
        let raw = unsafe { pw_sys::pw_filter_new(core.as_raw_ptr(), name.as_ptr(), node_props.into_raw()) };
        if raw.is_null() {
            return Err("pw_filter_new failed".to_owned());
        }
        let state = Box::into_raw(Box::new(State {
            inputs: [ptr::null_mut(); 2],
            outputs: [ptr::null_mut(); 2],
            silence: vec![0.0; MAX_FRAMES].into_boxed_slice(),
            scratch: [vec![0.0; MAX_FRAMES].into_boxed_slice(), vec![0.0; MAX_FRAMES].into_boxed_slice()],
            process,
        }));
        let mut events: Box<pw_sys::pw_filter_events> = Box::new(unsafe { std::mem::zeroed() });
        events.version = pw_sys::PW_VERSION_FILTER_EVENTS;
        events.process = Some(on_process);
        let mut hook: Box<spa_sys::spa_hook> = Box::new(unsafe { std::mem::zeroed() });
        unsafe { pw_sys::pw_filter_add_listener(raw, &mut *hook, &*events, state as *mut c_void) };
        let filter = Self { raw, state, _hook: hook, _events: events };

        for (dir, names) in [(spa_sys::SPA_DIRECTION_INPUT, ports.0), (spa_sys::SPA_DIRECTION_OUTPUT, ports.1)] {
            for (i, (port_name, channel)) in names.iter().zip(["FL", "FR"]).enumerate() {
                let props = properties! {
                    "format.dsp" => "32 bit float mono audio",
                    "port.name" => *port_name,
                    "audio.channel" => channel,
                };
                let port = unsafe {
                    pw_sys::pw_filter_add_port(
                        raw,
                        dir,
                        pw_sys::pw_filter_port_flags_PW_FILTER_PORT_FLAG_MAP_BUFFERS,
                        std::mem::size_of::<u64>(),
                        props.into_raw(),
                        ptr::null_mut(),
                        0,
                    )
                };
                if port.is_null() {
                    return Err(format!("pw_filter_add_port failed for {port_name}"));
                }
                // Not connected yet, so the realtime thread can't be reading this.
                let state = unsafe { &mut *filter.state };
                if dir == spa_sys::SPA_DIRECTION_INPUT {
                    state.inputs[i] = port;
                } else {
                    state.outputs[i] = port;
                }
            }
        }

        let res = unsafe { pw_sys::pw_filter_connect(raw, pw_sys::pw_filter_flags_PW_FILTER_FLAG_RT_PROCESS, ptr::null_mut(), 0) };
        if res < 0 {
            return Err(format!("pw_filter_connect failed: {res}"));
        }
        // No node-id getter here: `pw_filter_get_node_id` is PipeWire 1.1+,
        // newer than CI's and the AppImage's. `Engine` finds the node by its
        // `node.name` in the registry instead.
        Ok(filter)
    }
}

impl Drop for DspFilter {
    fn drop(&mut self) {
        // Destroy first: it stops the realtime callback and unhooks our
        // listener, so `state` is unused once it returns.
        unsafe {
            pw_sys::pw_filter_destroy(self.raw);
            drop(Box::from_raw(self.state));
        }
    }
}
