# Testing

Fonyx is tested at five levels:

1. **Unit tests** in the daemon, for DSP and pure logic, and in the CLI.
2. **Endpoint tests** that call every command the frontend can use, against
   a real daemon.
3. **UI smoke tests** with Playwright, driving the real page.
4. **CLI tests** that run every `fonyx` command against a real daemon.
5. **Manual verification against real audio**, recorded below as
   engineering notes.

Levels 1 to 4 run in CI on every pull request. See
[Continuous integration](#continuous-integration).

## Daemon unit tests

```bash
cargo test -p fonyx-daemon
```

84 tests, none of which need PipeWire or D-Bus. They cover code that was
written as pure functions for this reason.

| Module | Tests | What is checked |
|---|---|---|
| `dsp::smoothed` | 2 | Values ramp rather than jump; works for non-unit ranges |
| `dsp::biquad` | 3 | Low-pass attenuation, near-identity at 0 dB, coefficient updates keep filter history |
| `dsp::dynamics` | 4 | Compressor reduces a loud signal, gate closes after hold, limiter holds the ceiling and leaves quiet signals alone |
| `dsp::delay` | 4 | Pass-through at zero, correct delay, gradual slew, clamp at maximum |
| `dsp::spectrum` | 2 | A sine peaks in the right bin; silence stays at the floor |
| `plugins` | 9 | Registry contents, each plugin's effect, chain order and bypass, unknown plugin types are skipped |
| `pw` | 5 | Pan law, service-name extraction, app-id matching returns every match, metrics round trip |
| `output` | 9 | Default-output policy and delay resolution in every sync mode |
| `identity` | 11 | App and output identity fallbacks, output kind, latency parsing |
| `rules` | 10 | Trigger matching, conditions, priority, per-stream cooldown, validation, bounded history |
| `ducking` | 4 | Ducks on a loud trigger, not on a quiet one, holds before releasing, combines simultaneous triggers |
| `scenes` | 2 | Stream placement planning |
| `config` | 4 | TOML round trip with every feature populated; Cue group creation and routing; one-level sub-group rule; profile cleanup after group deletion |
| `doctor` | 6 | Each health check and the overall score |
| `graph` | 6 | Signal-flow graph construction |
| `recording` | 3 | A valid WAV file is produced; FLAC round-trips through an independent decoder (`claxon`); temporary-file naming |

The engine's PipeWire interaction (`pw.rs` outside its pure helpers) and the
D-Bus layer have no unit tests. They are covered by the smoke tests and by
manual verification.

## The isolated test session

```bash
cargo build -p fonyx-daemon
cd ui/tests && npm ci && npx playwright install chromium && cd ../..

sh ui/tests/run-isolated.sh              # everything
sh ui/tests/run-isolated.sh api.spec.mjs # one file; extra arguments go to Playwright
```

`ui/tests/run-isolated.sh` builds a throwaway environment and runs the
endpoint and UI tests inside it:

- a private D-Bus session bus with service activation switched off
- its own PipeWire and WirePlumber, in a temporary runtime directory, with
  WirePlumber's hardware monitors disabled
- one fake output device (`test-output`) and two fake applications
  (`TestAppAlpha`, `TestAppBeta`), created as PipeWire null nodes
- the real `fonyx-daemon` with a temporary config directory

Nothing in it touches the machine's real audio session, devices or
`config.toml`, and everything is removed afterwards. CI runs the same
script. This is the recommended way to run the suite.

## Endpoint tests

`ui/tests/api.spec.mjs`, 24 tests. They only run inside the isolated
session, because they route, mute and change the volume of streams and
outputs.

Each test calls Tauri commands by name through the test bridge, exactly as
the frontend's `invoke()` does, and checks the result against the daemon's
state. Between them they call **every** command in `commands.rs`:

| Area | What is checked |
|---|---|
| Groups | Create, list, gain, delete; Default and Cue cannot be deleted; master gain |
| Sub-groups | Feed, refusal of a second level, release on parent delete |
| DSP | EQ band, filters, compressor and gate read back through `get_group_details`; EQ level and pan clamping |
| Plugins | Available types, add, parameter, bypass, reorder, remove |
| Live data | Shapes of levels, spectrum, metrics, health checks and graph |
| Streams | Discovery, auto-routing into Default, routing, left/right isolate, mute, unroute, return to Default when a group is deleted, profile save, Own DSP |
| Outputs | Discovery, device volume and mute, group routes, Master routes, sync modes, preferred device, buffer and compensation clamping |
| Cue | Cue group, cue outputs, duck level |
| Scenes | Capture, activate (restores a changed gain), update, duplicate, delete |
| Rules | Save, read, list, disable, history, validation errors, delete; the two template rules |
| Ducking | Create, list, disable, delete |
| Recording | WAV 16-bit, WAV float and FLAC each write a file; bad format and unknown group are refused |

Three contract tests keep the layers in step:

- every Tauri command has a D-Bus method, and the D-Bus methods with no
  command are an explicit list (`CreateAutoRouteRule`,
  `CreateDeviceConnectedRule`, `SetGroupFeeds`), which are tested directly
  over D-Bus
- every command is registered in Tauri's handler list, and the frontend
  only invokes commands that exist
- every command was exercised by this file, so a new command without a test
  fails the build

The two autostart setters are exempt from the last check: they flip systemd
units and the isolated session has no systemd user instance.

## UI smoke tests

`ui/tests/smoke.spec.mjs`, 13 tests. They run in the isolated session, and
can also be run against a daemon you already have running:

```bash
cd ui/tests && npm test
```

### These are not mocked

The tests load the real `ui/index.html` in headless Chromium. Outside Tauri
there is no `window.__TAURI__`, so `ui/tests/bridge.mjs` supplies one: it
serves the page over HTTP on port 4571 and forwards every `invoke()` call
to the `org.fonyx.Daemon` service on the session bus.

The mapping from Tauri command to D-Bus method is generated by parsing
`ui/src-tauri/src/commands.rs`, so it cannot drift from the real command
list.

The consequence is that the tests exercise a real daemon: a test that
creates a group creates a real PipeWire sink, and a recording test writes a
real file. What they do not exercise is the Tauri Rust backend itself or the
WebKitGTK webview, since the page runs in Chromium through the bridge.

The two autostart commands do not go through the daemon. The bridge mirrors
`settings.rs` and calls `systemctl` directly for them.

### Coverage

| Area | Test |
|---|---|
| Navigation | Every tab renders its section |
| Home | Shows real group data from the daemon |
| Groups | Create and delete a group |
| Rules | Build a site-filtered rule, edit it, delete it |
| Scenes | Capture, activate, delete |
| Ducking | Create a rule between two groups, delete it |
| Plugins | Add, bypass, remove |
| Recording | Start, confirm the file exists on disk, stop, clean up |
| MTX Panel tab | Drag an EQ node with the pointer and confirm the daemon's stored frequency changed |
| Settings | Theme; preferred output list; buffer size read and write; Quickshell autostart through systemd (skipped in the isolated session) |

### Against a real daemon

Tests run serially because they share one daemon. Every smoke test that
changes state deletes or restores what it touched. If you run them outside
the isolated session they act on your real daemon and config, so a
transient group or rule will appear for a moment.

## CLI tests

```bash
cargo build -p fonyx-daemon -p fonyx-cli
FONYX_TEST_CMD="sh cli/tests/smoke.sh" sh ui/tests/run-isolated.sh
```

`cli/tests/smoke.sh` runs the `fonyx` binary against the daemon in the
isolated session: each command, the output it should print, and the mistakes
it should refuse (deleting Default, a vague name, `75` for `75%`, a plugin
slot that is not there). `run-isolated.sh` runs `FONYX_TEST_CMD` in
place of Playwright when it is set, so the script needs no Node. It needs
`jq`.

The `system start`, `stop`, `restart` and `logs` commands are not covered:
they act on the user's real systemd, which the isolated session has none of.

The CLI's pure logic (name and alias resolution, level parsing, the text
layout) has unit tests in `cli/src/`.

## Continuous integration

`.github/workflows/ci.yml` runs on every push to `staging` or `main` and on
every pull request into either, on an Ubuntu 24.04 runner:

1. Install build dependencies and PipeWire.
2. `cargo build --workspace --locked`
3. `cargo test --workspace --locked`
4. Install Node dependencies and Chromium.
5. `sh ui/tests/run-isolated.sh`
6. `FONYX_TEST_CMD="sh cli/tests/smoke.sh" sh ui/tests/run-isolated.sh`
7. `npm ci && npm run build` in `site/`, which type-checks and builds the
   website.

Work lands on `staging` first. `main` is protected: it only takes pull
requests, the "Build and test" check must pass, and that check fails any
pull request into `main` that does not come from `staging`. See
[Development](development.md#contributing-changes).

## Known test gaps

- The endpoint tests check that each call is stored and reported correctly.
  They do not listen to audio: nothing verifies what a filter, the
  compressor or a pan setting does to a real signal beyond the DSP unit
  tests.
- The fake applications are silent, so levels, ducking triggers and
  underrun behaviour are not exercised with signal.
- No UI tests for the Outputs tab, the cue bus, pan and mono/stereo, the
  left/right button, fader groups or keyboard control.
- No tests for the Quickshell surfaces.
- No test of the Tauri backend binary or of rendering in WebKitGTK.
- No lint or formatting check in CI.
- CI runs on one distribution (Ubuntu 24.04).

## Engineering verification

Much of the engine was verified by hand against a real PipeWire session with
real application audio. The findings below are the ones that shaped the
design. Tools used: `pw-link -l`, `pw-top`, `wpctl status`, `busctl`,
generated test tones, and a headless browser driving the UI.

These are observations from development on one machine. They are evidence
that the mechanisms work as described, not benchmarks.

### Group nodes and routing

- Each group appears as a real `Audio/Sink` node, selectable in
  `wpctl status`.
- `RouteStreamToGroup` links a stream's ports into the group sink port by
  port, channel-matched. Confirmed with `pw-link -l`.
- With a tone routed through a group and an EQ band and gain change applied
  mid-stream, `pw-top` showed both of the group's stream nodes at `ERR 0`.

### Routing isolation

**Problem.** Routing a stream into a group originally left the stream's
direct link to hardware in place. The signal reached the device twice: once
directly, and once through the group's processing and ring-buffer latency.
A user reported this as an audible oscillating, phasing sound, which is what
a comb filter from two delayed copies produces.

**Fix.** The engine tracks every link in the graph and destroys a stream's
existing playback links before creating the new one. If PipeWire's session
manager later relinks a routed stream to hardware, the engine removes that
link too.

**Verification.** `pw-link -l` before and after: routing removed the direct
hardware link and left only the group link.

**Later change.** Removing a stream from a group originally reconnected it
to hardware. That left the stream outside the mixer and invisible to the
faders. Removal now moves it into the Default group; reconnecting to
hardware happens only at daemon shutdown.

### Routing survives restarts

**Problem.** PipeWire replays every live stream as a new global when the
daemon reconnects. Auto-routing each one unconditionally swept every stream,
including ones the user had placed deliberately, back into Default on every
restart.

**Fix.** A stream already listed under a group is returned to that group.
Stream ids that closed while the daemon was down are pruned after the
registry's first full replay, because PipeWire reuses ids.

### Port renegotiation

**Problem.** Some applications replace a stream's ports on pause and resume
without recreating the node. The old links vanished with the old ports, and
the new ports sat unlinked: the video kept playing with no audio.

**Fix.** The engine remembers each routed stream's group and re-applies the
route whenever a new port appears on that stream.

**Related.** Pairing a single early port by position caused audio in one
channel only. Routing now waits for a full stereo port set for the first few
attempts.

### Identifying streams

Most applications do not expose a per-tab title on their PipeWire stream.
Several browser tabs appeared as identical rows. The daemon now tracks each
node's `RUNNING` state so the UI can show which stream is playing, extracts
a service name from `media.name` where one is set, and the UI numbers
same-named streams.

A stream's group membership is server state returned by `ListGroups`, not a
client-side list, so a routed stream leaves the Inputs list and appears
under its group in every client.

### D-Bus and the UI path

- Every method was exercised with `busctl --user call` and confirmed to
  update both `config.toml` and the running engine.
- The Tauri UI was run against the live daemon and confirmed to render real
  data on every tab.
- The MTX Panel tab was opened on a group that already had an EQ band enabled
  from an earlier session. It drew that group's real +6 dB / 1 kHz curve
  rather than a flat line, confirming controls are populated from the
  daemon.
- A real pointer drag on an EQ node in a headless browser changed both the
  node's position on screen and the frequency the daemon reported from
  `GetGroupDetails`. This is now a smoke test.

### Meters and metrics

- CPU and RAM come from `sysinfo`; buffer size is the size PipeWire actually
  delivers; the underrun counter is the engine's own.
- Routing a generated tone through a group produced non-zero peak levels in
  `GetLevels` that tracked the signal. Idle groups read zero underruns.
- **Compressor marker.** With a tremolo-modulated tone, the marker on the
  compressor graph moved frame to frame with the signal, and changed colour
  with gain reduction of −12 to −24 dB once the gain was raised enough to
  engage the compressor on a real group's settings.

### Safety limiter

**Problem.** A real configuration (six boosted EQ bands plus a compressor
with +6.14 dB makeup gain) produced output above 0 dBFS for loud input,
heard as distortion on loud passages. A hard clamp prevents overflow but
sounds like clipping whenever it engages.

**Fix.** An always-on limiter with a −0.3 dBFS ceiling, ahead of the clamp.

**Verification.** With those settings and the group gain temporarily doubled
to force the overshoot, output held at about 0.977 peak (roughly −0.2 dBFS)
instead of pinning at 1.0.

### Ring-buffer underruns

The capture-to-playback ring buffer went through four rounds of
investigation. Each fixed a distinct, observed fault.

**1. The underrun gate asked the wrong question.** Whether an empty ring was
a fault was first inferred from control-plane state: "is a stream routed
here", then "is it in the `Running` state". Both are wrong the same way. A
stream can sit at `Running` while producing nothing, and a stream that ends
leaves a gap before its removal is processed. Observed: a burst of about
76,800 samples counted in under a second as a test stream ended, and a
routed browser tab logging callbacks that were 100% misses while marked
active.

*Fix:* the capture callback increments a counter each cycle it receives a
buffer. Playback checks that counter, with a three-cycle grace period
because the two streams have no ordering guarantee.

**2. Playback popped more than capture supplied.** The playback callback
filled the buffer's full capacity rather than the number of frames
requested for the cycle. When the negotiated buffer was larger than one
quantum, the ring drained faster than it filled, heard as looping and
cutting out.

*Fix:* produce the requested frame count.

**3. Too little slack.** Rare single-quantum misses remained during clean
playback. A new playback stream raced capture and settled with only about
16 ms of the ring's roughly 170 ms in hand, thin enough for one
scheduling-order flip to drain it.

*Fix:* a warm-up. Playback outputs silence, not counted as underrun, until
the ring holds a threshold amount. Verified with more than 90 seconds of
continuous playback at zero underruns.

**4. The warm-up did not re-arm.** The warm-up ran once. Any ordinary
silence (a paused video, an advert break) drained the ring to empty, and
playback then resumed with no slack for the rest of the session. A daemon
left running through about 4.5 hours of normal browser use reached over
5.2 million underruns at a sustained rate near 60,000 per second, audible as
distortion.

*Fix:* re-enter warm-up when occupancy falls below a quarter of the
threshold.

*Verification:* a real source process was suspended with `SIGSTOP` until the
ring passed the low watermark, then resumed, three times, with zero
underruns throughout. Feeding silence would not have exercised the fault,
because a stream fed silence keeps ticking.

The warm-up threshold later became the user-configurable buffer size.

### Threading

**Problem.** The engine was first written on the assumption that all its
code, including audio callbacks, ran on one thread, and shared state through
`Rc<RefCell<_>>`. Streams connected with `RT_PROCESS` run their callbacks on
PipeWire's realtime thread. This produced an `already borrowed` panic under
real audio load.

**Fix.** State shared with a callback is either atomic or behind
`Arc<Mutex<_>>`.

### Drop order of PipeWire objects

**Problem.** The daemon occasionally segfaulted in CI while a group or route
was being torn down, on the same commit that passed elsewhere. Every struct
that owned a stream or proxy declared it before its listener, so the stream
was destroyed first; `pipewire-rs` then unhooks the listener unconditionally,
writing into the freed stream. Whether that corrupted anything depended on
what reused the memory, hence the rarity.

**Fix.** Listeners are declared, and so dropped, before the object they
listen to.

**Finding it.** An address-sanitizer build reports this class of bug at once,
where 600 stress cycles on a normal build passed:

```bash
RUSTC_BOOTSTRAP=1 RUSTFLAGS=-Zsanitizer=address \
  cargo build -p fonyx-daemon --target x86_64-unknown-linux-gnu --target-dir /tmp/sf-asan
ASAN_OPTIONS=detect_leaks=0 \
  FONYX_DAEMON=/tmp/sf-asan/x86_64-unknown-linux-gnu/debug/fonyx-daemon \
  sh ui/tests/run-isolated.sh
```

A report, if any, is in the daemon's log, which the runner prints when a
test fails.

### Shutdown, hangs and feedback

- **Shutdown.** Without signal masking, SIGTERM could kill the process on an
  arbitrary thread before the engine reconnected streams, leaving
  applications silent. Signals are now blocked on all threads and handled on
  the engine thread.
- **Hangs.** A deadlock once left the engine thread stuck with the process
  alive. The systemd watchdog ping is sent from that thread so that such a
  hang triggers a restart.
- **Feedback loop.** With every hardware sink gone, the session manager made
  one of the daemon's own sinks the system default. A group's implicit route
  then fed the Default group into itself, pinned near 0 dBFS. The engine now
  refuses to autoconnect an implicit route to its own sinks.
- **Microphone indicator.** The per-stream meter taps are capture streams,
  and desktop privacy indicators reported microphone use each time an
  application started or stopped playing. They are now flagged as sink
  captures.

### Per-stream left / right isolate

Verified on a live browser stream by switching the setting over D-Bus and
reading the resulting links with `pw-link`: across 18 transitions between
left, right and stereo, both of the stream's ports were linked to the
expected channel of the group sink each time.

### Quickshell

Both the widget and the standalone app were run under real `quickshell`
against the live daemon. The widget parsed a real `ListGroups` reply into
group rows; the app opened its window, showed a correct connection status,
and drew EQ and compressor data matching the daemon's stored state.
