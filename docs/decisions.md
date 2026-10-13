# Design Decisions

Decisions that shaped the project, each with its reasoning, its cost and
where it stands. Several resolve items the
[project specification](../PROJECT_SPEC.md) left open.

## Daemon and thin clients

**Decision.** All audio, routing and persistence live in one daemon. The
Tauri UI and the Quickshell surfaces only call its D-Bus interface.

**Why.** Routing must keep working with no window open, and several clients
must show the same state.

**Trade-off.** Every UI feature needs a D-Bus method. Features implemented
purely in one client (mute, solo, fader groups in the Tauri UI) are not
shared with the others.

**Status.** In place.

## Configuration as TOML

**Decision.** One file, `~/.config/fonyx/config.toml`, honouring
`XDG_CONFIG_HOME`. Written atomically (temporary file, sync, rename) on
every change.

**Why.** A per-user settings file does not need a database. TOML is readable
and editable by hand, which helps debugging.

**Trade-off.** The whole file is rewritten and synced on each setter call,
including each step of a fader drag.

**Status.** In place. Resolves the specification's open item on config
format and location.

## Fixed 48 kHz stereo float format

**Decision.** Every stream the daemon creates is 48 kHz, two channels,
F32LE. The engine does not follow the graph's rate or the application's.

**Why.** One format keeps filter coefficients, ring-buffer sizing and delay
lines simple and identical everywhere.

**Trade-off.** PipeWire converts at the boundaries when the graph or an
application uses another rate or layout. The engine itself only ever
processes two channels.

**Status.** In place. Resolves the specification's open item on sample-rate
handling.

## Stable identities instead of node ids

**Decision.** Configuration refers to applications and outputs by stable
strings: an app identity from `application.id`, then `application.name`,
then `node.name`; an output identity from device serial, Bluetooth address,
vendor/product/card, then name.

**Why.** PipeWire node ids change across application restarts, device
reconnects and PipeWire restarts.

**Trade-off.** A browser's tabs share one app identity, so app-level
features cannot tell them apart. Rules can match on the extracted service
name to work around this.

**Status.** In place.

## A default group for everything

**Decision.** A protected Default group always exists, and every new stream
is routed into it.

**Why.** Nothing should play outside the mixer. The user opts a stream out
into its own group rather than opting each application in.

**Trade-off.** All desktop audio passes through the daemon's ring buffer and
its added latency. If the daemon stops, it must hand streams back to
hardware, which it does on shutdown.

**Status.** In place.

## Hand-linked ports

**Decision.** Streams are linked into group sinks port by port, matched by
channel.

**Why.** Node-to-node linking made PipeWire try to create new ports on
nodes with a fixed, negotiated format, and failed with "no more port
available".

**Trade-off.** The engine must track ports and links itself, retry while
ports are still being announced, and re-link when an application replaces
its ports.

**Status.** In place. The same mechanism provides per-stream left/right
isolate.

## Separate capture and playback streams with a ring buffer

**Decision.** A group is a capture stream plus one playback stream per
output route, joined by lock-free ring buffers.

**Why.** It allows a group to feed several outputs, each with its own gain
and delay, and lets Master and recordings tap the same signal.

**Trade-off.** The two sides are scheduled independently, so the engine
needs its own cushion: a warm-up threshold, a re-arm watermark and a
liveness check. The cushion (85 ms by default) is added latency.

**Status.** In place. See
[Testing](testing.md#ring-buffer-underruns) for how it reached this form.

## Underrun metric counts our own ring

**Decision.** The "underruns" figure counts the capture-to-playback ring
starving while capture is active. It is not PipeWire's driver xrun count.

**Why.** It is a real, locally measured signal that distinguishes "audio is
glitching" from "it is not". The driver's count needs a profiler
subscription the engine does not have.

**Trade-off.** It will not match the numbers `pw-top` reports.

**Status.** In place. A profiler listener is the upgrade path if the
distinction matters.

## Locks on structural state

**Decision.** Per-sample parameters are lock-free atomics. A few structural
collections shared with the audio callback (route fan-out, plugin chain,
recording taps, delay lines) are behind `Arc<Mutex<_>>`.

**Why.** Audio callbacks run on PipeWire's realtime thread, not the engine
thread. The first design shared these through `Rc<RefCell<_>>` and panicked
under load. The collections change only on infrequent control events and
the locks are held briefly.

**Trade-off.** This departs from the specification's rule of no locks in
the callback. A control operation can delay a callback by the time it holds
the lock.

**Status.** In place. A lock-free message queue would remove it.

## One engine thread

**Decision.** Registry events, commands and timers all run on one PipeWire
main-loop thread.

**Why.** `Engine` needs no internal locking, and every state change has one
place it can happen.

**Trade-off.** A slow control operation delays everything else on that
thread. A deadlock once left it stuck with the process still alive; the
systemd watchdog now catches a hang, and `wpctl` calls are made without
holding the config lock.

**Status.** In place.

## Shelling out to `wpctl`

**Decision.** Stream volume and mute, and output device volume and mute, are
set by running `wpctl`, with a two-second timeout.

**Why.** These act on PipeWire objects the daemon does not own. `wpctl` is
the tool already trusted for it, and it avoids implementing parameter
setting for foreign nodes.

**Trade-off.** A runtime dependency on WirePlumber's tool, and a process
spawn per change.

**Status.** In place.

## Smoothing everything

**Decision.** Every user-driven parameter ramps over 15 ms with a one-pole
curve. Filter coefficients are recomputed once per buffer from the smoothed
values.

**Why.** Instant changes click. A one-pole ramp is scale-invariant, so the
same time constant suits a gain and a frequency, and it cannot overshoot.

**Trade-off.** Coefficients step once per buffer rather than per sample.

**Status.** In place.

## An always-on limiter

**Decision.** Every group ends with a fixed limiter at −0.3 dBFS followed by
a hard clamp. Neither is configurable.

**Why.** No earlier stage knows what the others added. Real settings
produced output above full scale, and a clamp alone sounds like clipping.

**Trade-off.** The limiter has no lookahead, so the clamp still catches
instantaneous steps. Users cannot disable it.

**Status.** In place.

## Balance-style pan

**Decision.** Pan attenuates the opposite channel and leaves the other at
unity. Mono is the average of both channels.

**Why.** At center the signal is untouched, so adding the control changed
nothing for existing groups.

**Trade-off.** It is not a constant-power pan: a signal panned hard to one
side is not boosted to compensate, and with stereo content the far
channel's material is lost rather than moved.

**Status.** In place.

## Fader linking for "global fader configuration"

**Decision.** The specification's undefined "global" fader mode is
implemented as linking: linked faders move by the same amount together.

**Why.** The master fader already applies one offset to everything, so
linking is the reading that adds something distinct.

**Trade-off.** Links are stored in the Tauri UI only.

**Status.** In place, later joined by fader groups.

## Processing per group, not per stream

**Decision.** DSP, recording and profiles work on groups. An application
that needs its own processing gets its own group ("Own DSP"), created as a
copy of the group it was in.

**Why.** One chain per group keeps the engine simple, and a group of one is
exactly a per-app chain.

**Trade-off.** Recording an application that shares a group records the
group's mix.

**Status.** In place. An earlier form nested the new group under its parent
(`feeds_group`); nesting remains supported, one level deep.

## In-process plugins only

**Decision.** Plugins are Rust types implementing one trait, compiled into
the daemon.

**Why.** It settles the chain abstraction (parameters, bypass, ordering,
persistence) before taking on external formats.

**Trade-off.** No third-party plugins.

**Status.** In place. LV2 and VST hosting are not implemented.

## Rules as data

**Decision.** Rules are a fixed set of triggers, conditions and actions.
They cannot run commands.

**Why.** Safety, and loop freedom: no action raises an event that could
trigger a rule again.

**Trade-off.** Less flexible than a scripting hook.

**Status.** In place.

## Ducking on a timer

**Decision.** Ducking is evaluated every 20 ms on the engine thread, from
each group's pre-fader input peak.

**Why.** Ducking one group depends on another group's level. A timer on the
engine thread can see all groups without shared mutable state between audio
callbacks. Using the pre-fader peak means a trigger group's own fader does
not decide whether others duck.

**Trade-off.** Envelope timing is accurate to 20 ms, not to the sample.

**Status.** In place.

## Quickshell through `busctl`

**Decision.** The Quickshell surfaces call the daemon by spawning `busctl`
and parsing its JSON output.

**Why.** Quickshell has no QML type for arbitrary D-Bus calls, only
protocol-specific wrappers.

**Trade-off.** A process per call, which forces slower polling than the
Tauri UI.

**Status.** In place. See [Quickshell](quickshell.md).

## Polling instead of signals

**Decision.** Clients poll. The daemon emits change signals, but no shipped
client subscribes.

**Why.** Polling was the simplest path to a working UI, and the Quickshell
mechanism cannot subscribe at all.

**Trade-off.** Up to five seconds before one client shows another's change.

**Status.** In place. Subscribing in the Tauri backend is possible future
work.

## `pipewire` crate 0.10

**Decision.** The daemon uses `pipewire = "0.10"`.

**Why.** On the development system (PipeWire/SPA headers 1.6.8), the 0.8
bindings did not build correctly: bindgen produced opaque fields for
`spa_pod_builder`. Version 0.10.1 built cleanly.

**Status.** In place.
