# Audio Engine

The engine lives in `daemon/src/pw.rs`. It connects to PipeWire, tracks
application streams and output devices, and owns the nodes that make up each
group.

## PipeWire objects

For each group the daemon creates:

| Object | PipeWire identity | Purpose |
|---|---|---|
| Capture stream | `media.class=Audio/Sink`, `node.name=sonicforge.group.<id>` | The virtual sink applications are linked into. Its process callback runs the DSP chain. |
| Route playback stream(s) | `node.name=sonicforge.<label>.route.<output or default>` | One per output route. Reads the processed signal and plays it to a device. |

For each application stream it also creates a passive capture tap
(`sonicforge.meter.<stream id>`) that only measures the stream's peak level
for the Inputs tab. The tap is flagged `stream.capture.sink` so desktop
privacy indicators do not report microphone use.

Group sinks show up in `wpctl status` as ordinary selectable sinks.

## Audio format

Every stream the daemon creates is fixed at **48 kHz, stereo, 32-bit float
(F32LE)**. The engine does not negotiate the graph's native rate; PipeWire
converts at the edges. See [Design decisions](decisions.md).

## Stream discovery

The engine listens to the PipeWire registry. A node is treated as an
application stream when it is an audio output stream that is not one of the
daemon's own. For each one the engine records:

- its name and a stable app identity (`identity::app_identity`)
- a service name extracted from `media.name` where the application sets one
  (for example a browser tab's site)
- whether the node is currently `RUNNING`, tracked through a node proxy
- a live peak level from its meter tap

Ports and links are tracked as separate registry globals. PipeWire announces
a node before its ports, so a route requested the moment a stream appears
may find no ports yet. Such routes are kept as pending and retried when a
port arrives for that node.

## Routing a stream into a group

`RouteStreamToGroup` does three things on the engine thread:

1. **Severs existing playback links.** Any link from the stream into an
   output device or another group is destroyed. Links into listeners (meter
   taps, recorders, another mixer's meters) are left alone.
2. **Links the stream's ports into the group sink**, port by port, matched
   by channel label (`FL` to `FL`, `FR` to `FR`). Node-to-node linking was
   tried and rejected: PipeWire attempted to create new ports and failed
   with "no more port available".
3. **Records the assignment** so it can be redone. Some applications replace
   their ports on pause/resume without recreating the node; when a new port
   appears on an already-routed stream, the route is applied again.

If the session manager later links a routed stream straight back to hardware,
the engine sees the new link and removes it.

The first few attempts wait for a full stereo port set on both sides, to
avoid pairing a lone early port with the wrong channel. After five attempts
(`MAX_STRICT_ROUTE_ATTEMPTS`) a single port is accepted, so a genuinely mono
source is still routed.

### Left / right isolate

A stream can be forced to one side with `SetStreamSide`. Every port of the
stream is then linked into that one channel of the group sink instead of its
own. The setting is held in memory for the life of the stream.

### Removing a stream from a group

`UnrouteStream` moves the stream back into the Default group. It does not
reconnect it to hardware: every stream is meant to stay inside the mixer.
Reconnecting to hardware happens only when the daemon shuts down.

### Default routing

A stream that has no assignment is routed into the Default group. On daemon
restart PipeWire replays every live stream; those already listed under a
group are returned to that group rather than swept into Default.

When a manual move is made, it also becomes the app's preferred group in its
profile, so a browser that opens a new stream on each play lands in the same
group.

## Group signal path

![Audio signal flow through one group](images/signal-flow.svg)

Capture and playback are separate PipeWire streams with no link between
them in the graph. The only connection is a ring buffer per consumer
(`rtrb`, lock-free, single producer and single consumer).

## Ring buffer and underrun handling

Each ring holds `RING_CAPACITY = 16384` samples, about 170 ms of stereo
audio at 48 kHz.

Because capture and playback are scheduled independently, the playback side
manages its own cushion:

- **Warm-up.** A new route plays silence until the ring holds a threshold
  amount, then starts reading. The threshold is the Settings "buffer size"
  (`buffer_ms`, default 85 ms, clamped to 10–150 ms) and is fixed when the
  route is created.
- **Re-arm.** If occupancy falls below a quarter of the threshold, the route
  goes back into warm-up. Ordinary silence (a paused video) drains the ring;
  without re-arming, playback would resume with no slack.
- **Request size.** The playback callback produces the number of frames the
  graph requested this cycle, not the buffer's full capacity.
- **Liveness.** The capture callback increments a counter every cycle it
  receives a buffer. Playback treats capture as alive if that counter moved
  within the last three cycles.

An **underrun** is counted when the ring is empty while capture is alive.
An empty ring while capture is idle is silence, not a fault. The count is
exposed through `GetMetrics` and measures this ring starving; it is not
PipeWire's own driver xrun count.

The history behind these mechanisms is in
[Testing](testing.md#ring-buffer-underruns).

## Outputs

### Discovery and identity

Sinks and their parent devices are discovered from the registry. Each output
gets a stable id built from the most durable property available: device
serial, then Bluetooth address, then USB/PCI vendor, product and card, then
a name-based fallback. Routes are stored against this id, so they survive a
device being unplugged and reconnected.

Each output is classified (headphones, speaker, Bluetooth, USB, HDMI,
DisplayPort, virtual, other) and given an estimated latency when PipeWire
reports one.

### Output routes

A group's `output_routes` list holds one entry per device with gain, enable,
sync mode and manual delay. The engine reconciles live playback streams
against that list whenever it changes or a device comes or goes. A route
whose device is absent keeps its configuration and is recreated when the
device returns.

A group with no explicit routes behaves as follows:

| Situation | Where the group plays |
|---|---|
| No Master routes, no preferred output | One implicit route, connected by PipeWire's session manager to its default sink |
| A preferred output is set and available | That device |
| A preferred output is set but unavailable | Chosen by policy: headphones, then speakers, then the system default, then anything available |
| Master has routes | Through the Master bus only |
| The group feeds another group | Into that group's sink |
| It is the Cue group | The cue outputs only; silent if none are set |

If no hardware sink remains and the session manager makes one of the
daemon's own sinks the system default, the engine does not autoconnect to it.
That case produced a feedback loop in practice.

### Delay compensation

Each route has a delay line (`dsp::delay`), pre-allocated for up to 500 ms.
Its sync mode decides the delay applied:

| Mode | Delay |
|---|---|
| Off | None |
| Manual | The configured value, clamped to the maximum compensation |
| Automatic | The largest known latency among the group's automatic routes, minus this output's own. Zero if this output's latency is unknown. |

Delay changes are slewed and read with linear interpolation rather than
applied instantly. The maximum compensation defaults to 150 ms.

### Master and Cue

The Master route's playback callback is the mixing point: it reads one tap
per live group and sums them. Sub-groups and the Cue group are excluded.

While the Cue group is audible, routes onto cue outputs from Master and from
other groups are lowered to `cue_duck_level` (default 0.2). Other outputs
are unaffected.

## Timers and events on the engine thread

| Trigger | Action |
|---|---|
| Every 20 ms | Evaluate ducking rules against groups' input peaks |
| Every 2 s | Send the systemd watchdog ping |
| Stream, device or scene event | Evaluate rules |
| Stream appears | Apply its profile, pending scene assignment, or default routing |
| SIGTERM / SIGINT | Reconnect routed streams to hardware, then quit |

## Known limitations

- **Master fader ramp.** Every group's callback advances the shared master
  gain ramp once per frame, so with several active groups it converges
  faster than its nominal 15 ms. It stays smooth.
- **Master warm-up.** All of a Master route's inputs share one cushion.
- **Stream meters** require a full stereo port set; a mono-only stream gets
  no meter.
- **Preferred group is per app id.** Every tab of a browser shares one app
  identity, so the last manual move wins for all of them unless a rule
  matches by site.
- **Brief locks on the realtime path.** Structural lists are behind a mutex
  that the audio callback takes each buffer. Contention is limited to
  moments when the control thread is changing routes, plugins or recordings.
- **PipeWire restarts** are not handled.
