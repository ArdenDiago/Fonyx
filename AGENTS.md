# SonicForge: brief for AI agents

Read this first. It is a self-contained summary of the whole project. The
detail lives in `docs/` (start at `docs/index.md`); source comments are the
final authority.

## What it is

SonicForge is a per-application audio mixer and router for Linux, built on
PipeWire. Each application's audio is placed in a user-defined **group**.
Every group has its own fader and DSP chain (filters, EQ, compressor, gate,
plugins, pan) and can be sent to one or more output devices.

- Linux only. Version 0.1.0, no tagged release, no published packages.
- Developed on Arch Linux; CI runs on Ubuntu 24.04.
- License: GPL-3.0-or-later.
- Repository: <https://github.com/ArdenDiago/SonicForge>. Work lands on the `staging` branch; `main` only takes pull requests from `staging`.
- Formerly named "MTX Panel" / `mtxpanel`. Everything is now `sonicforge`
  in lower case. The only surviving old name is the EQ tab's title,
  "MTX Panel". Do not reintroduce the old name.

## Architecture in one paragraph

One background daemon owns the PipeWire graph, runs all DSP and persists all
state. Every client (Tauri desktop UI, `sonicforge` CLI, Quickshell QML
surfaces) is a thin client that calls the daemon's single D-Bus interface on
the session bus. Clients hold no audio or routing logic. Closing or crashing
a client never affects audio.

```text
Audio apps ──streams──▶ PipeWire ◀──nodes, ports, links──▶ sonicforge-daemon
                           │                                    ▲
                           ▼                                    │ D-Bus (session bus)
                     Output devices            org.sonicforge.Daemon
                                                                │
                              ┌───────────────┬─────────────────┼──────────────┐
                          Tauri UI        sonicforge CLI    Quickshell      tests
                       (sonicforge-ui)                     (via busctl)
```

## Repository map

Cargo workspace members: `daemon`, `client`, `cli`, `ui/src-tauri`.

| Path | Crate / binary | Contents |
|---|---|---|
| `daemon/` | `sonicforge-daemon` | PipeWire engine, DSP, D-Bus service, config |
| `daemon/src/main.rs` | | Startup, threads, signal masking, `sd_notify` |
| `daemon/src/pw.rs` | | The engine (~3,350 lines): registry tracking, group and route nodes, audio callbacks, `Command` enum |
| `daemon/src/dbus.rs` | | The `org.sonicforge.Daemon` interface (~1,560 lines, about 95 methods) |
| `daemon/src/config.rs` | | Config model, TOML load, atomic save |
| `daemon/src/group.rs` | | Group and output-route data model |
| `daemon/src/output.rs` | | Output model, default-output policy, delay resolution |
| `daemon/src/identity.rs` | | Stable app and output ids, output kind, latency estimate |
| `daemon/src/dsp/` | | `smoothed`, `biquad`, `dynamics`, `delay`, `spectrum` |
| `daemon/src/plugins.rs` | | `Plugin` trait, chain, four built-in plugins |
| `daemon/src/scenes.rs`, `profiles.rs`, `rules.rs`, `ducking.rs` | | Automation |
| `daemon/src/recording.rs`, `doctor.rs`, `graph.rs` | | WAV/FLAC recording, health checks, signal-flow graph |
| `client/src/lib.rs` | `sonicforge-client` | The `zbus` proxy trait for the daemon, shared by the UI and the CLI. Pure pass-through. |
| `cli/src/` | `sonicforge-cli`, binary `sonicforge` | `args.rs` (command tree), `main.rs` (commands), `resolve.rs` (names and aliases), `system.rs` (systemctl), `out.rs` (text and JSON output) |
| `cli/tests/smoke.sh` | | Runs every CLI command against a real daemon |
| `ui/index.html` | | The entire frontend: markup, CSS and JavaScript in one ~4,200-line file. No framework, no bundler, no build step. |
| `ui/src-tauri/src/` | `sonicforge-ui` | Tauri 2 backend: `lib.rs` (setup, command registration), `daemon.rs` (holds the shared proxy), `commands.rs` (about 80 one-line commands), `settings.rs` (autostart toggles via `systemctl --user`) |
| `ui/tests/` | | Playwright endpoint tests (`api.spec.mjs`), UI smoke tests (`smoke.spec.mjs`), the D-Bus bridge (`bridge.mjs`), the isolated runner (`run-isolated.sh`) |
| `quickshell/` | | QML: `SonicForgeApp.qml` + `app/` (standalone app), `SonicForgeWidget.qml` (bar widget), `SonicForgePopup.qml` (widget in a window) |
| `packaging/` | | `install-user.sh`, `systemd/` (three user units), `flatpak/` (manifest; never verified with a real `flatpak-builder` run) |
| `docs/` | | Full documentation, one page per area |
| `PROJECT_SPEC.md` | | The original build brief. The project has outgrown it; where it differs from `docs/` and the source, they win. |

Doc drift to be aware of: the module-map diagrams in `README.md` and
`docs/architecture.md` still place the D-Bus proxy in
`ui/src-tauri/src/daemon.rs`. It now lives in `client/src/lib.rs`.
`docs/packaging.md` does not mention `packaging/flatpak/`.

## Core concepts

- **Stream.** One application's audio output node in PipeWire. Identified
  live by PipeWire node id (valid only while the stream lives) and
  persistently by a stable app identity: `application.id`, else
  `application.name`, else `node.name`.
- **Group.** A user-defined bus: a virtual PipeWire sink
  (`sonicforge.group.<id>`) with its own DSP chain and fader. A stream is in
  exactly one group at a time.
- **Default group.** Protected, always exists. Every new stream lands here
  unless a profile, rule, scene or earlier assignment says otherwise, so no
  audio bypasses the mixer.
- **Cue group.** Protected, permanent, for soloing to headphones. Plays only
  on outputs marked as cue outputs and is never summed into Master. While it
  is audible, everything else on those outputs is lowered to
  `cue_duck_level` (default 0.2).
- **Output route.** A group's connection to one device, with its own gain,
  enable switch, sync mode and delay. A group with no explicit routes uses
  one implicit default route.
- **Master bus.** The sum of every group's processed signal except Cue and
  sub-groups, with its own route list. Silent until a route is added.
- **Sub-group.** A group can feed another group's input (`SetGroupFeeds`),
  one level deep only.
- **Output id.** A stable string built from device serial, then Bluetooth
  address, then vendor/product/card, then name. Routes are stored against
  it and survive a replug.

## Daemon internals

### Threads

| Thread | Runs | Touches |
|---|---|---|
| Engine (main) | PipeWire main loop: registry events, `Command` handling, 20 ms ducking timer, 2 s watchdog ping, signal handling | `Engine` state |
| Realtime | Every stream's `process` callback (`RT_PROCESS`) | DSP state, ring buffers |
| D-Bus | `zbus` blocking server | `Config`, shared read-only views |
| Metrics | `sysinfo` refresh once a second | CPU/RAM atomics |
| Resume watcher | logind `PrepareForSleep` | sends `SystemResume` |
| Recording writers | One per active recording | file I/O |

### Control flow for any change

```text
Client → D-Bus method
  → D-Bus thread updates Config and saves config.toml
  → D-Bus thread sends a Command over a PipeWire channel
    → Engine thread applies it to the graph or sets a parameter target
      → Realtime thread ramps to the new value
  → D-Bus thread emits a change signal
```

Read-only calls never touch the engine or wait on the audio thread; they
read `Config` or lock-free values the callbacks publish. Per-stream
volume/mute and device volume/mute bypass the engine and shell out to
`wpctl` (2 s timeout), because they act on objects the daemon does not own.

### Audio path

- Fixed format everywhere: **48 kHz, stereo, F32LE**. PipeWire converts at
  the edges.
- A group is a capture stream (the sink) plus one playback stream per output
  route (`sonicforge.<label>.route.<output or default>`). They are not
  linked in the graph; they are joined by a lock-free SPSC ring buffer
  (`rtrb`, 16,384 samples, about 170 ms).
- Playback manages its own cushion: a warm-up threshold (`buffer_ms`,
  default 85, clamped 10 to 150), re-arm when occupancy drops below a
  quarter of it, and a capture-liveness counter. An **underrun** is an empty
  ring while capture is alive. It is the engine's own count, not PipeWire's
  xrun count.
- Streams are linked into group sinks **port by port**, matched by channel
  label. Node-to-node linking fails with "no more port available".
- Routing a stream first destroys its existing playback links, otherwise
  the signal is heard twice (comb filtering). If the session manager relinks
  a routed stream to hardware, the engine removes that link.
- Ports are announced after their node, and some apps replace ports on
  pause/resume. Routes are kept pending, retried and re-applied when a port
  appears.
- Each stream also gets a passive meter tap (`sonicforge.meter.<stream id>`),
  flagged `stream.capture.sink` so desktops do not show a microphone
  indicator.
- On SIGTERM/SIGINT the engine thread reconnects every routed stream to
  hardware before exiting. Persisted assignments are kept for the next start.

### DSP chain, per group, in order

```text
Input (spectrum and input peak measured here)
 → Gain (fader × master × ducking × EQ level offset)
 → High-pass → Low-pass
 → EQ (six peaking bands)
 → Compressor + gate (stereo-linked)
 → Plugin chain (user-ordered: soft_clipper, stereo_widener, deep_bass, mids)
 → Mono / pan (balance-style)
 → Safety limiter (−0.3 dBFS, always on, no controls)
 → Hard clamp [−1, 1] (output meters read here)
 → Output routes, Master, recording
```

Each route then applies its own delay line, gain and enable switch. Every
user-facing parameter is a `Smoothed` value (atomic target and current,
one-pole ramp, 15 ms). Biquads are Direct Form I with RBJ cookbook
coefficients, rebuilt once per buffer without resetting filter history.
Only built-in plugins exist; there is no LV2/VST loading.

### Persistence

- `~/.config/sonicforge/config.toml` (honours `XDG_CONFIG_HOME`), written
  atomically (temp file, sync, rename) on every setter call.
- Holds groups with DSP settings and routes, master gain, scenes, profiles,
  rules, ducking rules, cue outputs, preferred output, buffer size, maximum
  delay compensation.
- **Not** in the daemon: group mute, solo, fader links, fader groups, custom
  EQ presets and theme live in the Tauri webview's local storage. Per-stream
  left/right isolate lasts only as long as the stream.
- CLI aliases live in `~/.config/sonicforge/aliases.toml`; the daemon and UI
  never read it.

## D-Bus interface

| | |
|---|---|
| Bus | Session |
| Service, interface | `org.sonicforge.Daemon` |
| Object path | `/org/sonicforge/Daemon` |

Defined in `daemon/src/dbus.rs`; method names are the PascalCase form of the
Rust function names. Full tables are in `docs/dbus.md`. Live signatures:

```bash
busctl --user introspect org.sonicforge.Daemon /org/sonicforge/Daemon
```

Conventions:

- Ids are `u32`. Rust `f32` travels as a D-Bus double.
- Gain is a linear multiplier (1.0 is unity) unless the name ends in `_db`.
- Structured replies are **positional tuples**, not dictionaries.
- "Not found" is in-band: `u32::MAX` for an id, an empty string or list for
  data.
- Every setter persists and updates the running engine.
- Signals are emitted on change, but no shipped client subscribes; all poll.
- No access control: any process in the user's session can call it. Rules
  are data and cannot run commands. `wpctl` arguments are never passed
  through a shell.

Method areas: groups and streams, levels and DSP, plugins, live data
(`GetLevels`, `GetSpectrum`, `GetMetrics`, `GetGraph`, `RunHealthChecks`),
outputs and routing, cue, scenes and profiles, rules, ducking, recording.

## Clients

**Tauri UI (`sonicforge-ui`).** Ten tabs: Home, Inputs, MTX Panel (EQ,
dynamics, plugins), Faders, Outputs, Scenes, Rules, Recording, Doctor,
Settings. Call path: `invoke('set_gain', { groupId, value })` in
`index.html` → `#[tauri::command]` in `commands.rs` → blocking `zbus` proxy
(3 s timeout) → daemon. Command names are snake_case, arguments camelCase.
It polls: `refreshAll` every 5 s, `pollLive` every 200 ms. It connects once
at startup, so the daemon must already be running.

**CLI (`sonicforge`).** `sonicforge [--json] [--quiet] <noun> <verb> [args]`.
Nouns: `system`, `group`, `input`, `fader`, `dsp`, `mtx`, `output`, `scene`,
`rule`, `duck`, `record`, `alias`. Things are named by id, exact name, name
prefix or substring, or alias; an ambiguous name stops with a candidate
list. Levels are written `75%`, `-6dB` or `0.75`; above +12 dB is refused.
Exit codes: 0 success, 1 failure, 2 usage error. Scripts should use
`--json`. Full reference: `docs/cli.md`.

**Quickshell.** QML has no generic D-Bus call type, so these surfaces spawn
`busctl --user --json=short call ...` per call and parse the JSON. They lag
the Tauri UI (no EQ level line, pan, mono/stereo, left/right isolate).

## Build, run, test

Requirements: Rust, `clang` (bindgen for `pipewire-rs`), `webkit2gtk-4.1`,
a PipeWire session with WirePlumber (`wpctl`), `busctl`. Optional:
`quickshell`, Node.js and `jq` for the tests. The `pipewire` crate is pinned
to 0.10 on purpose; 0.8 does not build against current headers.

```bash
cargo build --workspace
cargo test --workspace                       # unit tests, no PipeWire needed
cargo clippy --workspace

RUST_LOG=info cargo run -p sonicforge-daemon
cargo run -p sonicforge-ui                   # second terminal
cargo run -p sonicforge-cli -- group ls

# Integration tests in a throwaway PipeWire + D-Bus session (safe: never
# touches the real audio session or config)
cd ui/tests && npm ci && npx playwright install chromium && cd ../..   # once
cargo build -p sonicforge-daemon -p sonicforge-cli
sh ui/tests/run-isolated.sh                                            # endpoint + UI tests
SONICFORGE_TEST_CMD="sh cli/tests/smoke.sh" sh ui/tests/run-isolated.sh  # CLI tests

sh packaging/install-user.sh                 # release build to ~/.local/bin + systemd user units
```

Things that will bite you:

- Only one daemon can hold the bus name. Stop the installed one first:
  `systemctl --user stop sonicforge-daemon`.
- A development daemon uses the real config. To avoid that:
  `XDG_CONFIG_HOME=/tmp/sf-dev cargo run -p sonicforge-daemon`.
- Starting or stopping the daemon re-links every application's audio, an
  audible interruption on the developer's machine. Prefer the isolated
  session for anything automated.
- `cd ui/tests && npm test` outside the isolated session acts on the real
  daemon and config.

Workflow: work lands on `staging` first, by direct push or by a pull request
into it. CI (`.github/workflows/ci.yml`) runs on every push to `staging` or
`main` and every pull request into either: `cargo build --workspace --locked`,
`cargo test --workspace --locked`, the isolated endpoint and UI tests and the
CLI smoke test. When `staging` is green, a maintainer merges it into `main`
with a pull request. `main` is protected: the "Build and test" check must
pass, and it fails any pull request into `main` that is not from `staging`.
Changes touching only `docs/` or `README.md` skip the build steps.

## Rules for changing the code

### Audio callbacks (realtime thread)

- No allocation, logging, blocking or I/O in a `process` callback.
- Per-sample values come from atomics; anything the user can change while
  audio plays is a `Smoothed`.
- Never share state with a callback through `Rc<RefCell<_>>`. That caused a
  real `already borrowed` panic. Use atomics, or `Arc<Mutex<_>>` held
  briefly for structural lists.
- Recompute filter coefficients per buffer, not per sample, and never reset
  filter history on a parameter change.

### Engine and D-Bus

- Never hold the config lock across a `wpctl` call or any other blocking
  operation. That deadlocked the engine thread once.
- Never change the tuple shape of an existing D-Bus method. Three clients
  read tuples by position. Add a new method instead, as `ListGroupPans` and
  `ListStreamSides` do.
- New persisted fields need `#[serde(default)]` so existing config files
  still load.
- Never autoconnect an implicit route to one of the daemon's own sinks; it
  produces a feedback loop.

### Adding a control end to end

1. **Model.** Field on `Group` (`daemon/src/group.rs`) or `Config`, with
   `#[serde(default)]`.
2. **Engine.** `Command` variant in `pw.rs`, a live parameter, its use in
   the callback, and handling in `activate_scene` if scenes should carry it.
3. **D-Bus.** Setter in `dbus.rs` that updates `Config`, saves, sends the
   command and emits a signal; plus a getter or list method.
4. **Client crate.** Add the method to the proxy trait in
   `client/src/lib.rs`.
5. **Tauri.** Command in `commands.rs`, registered in `lib.rs`.
6. **Frontend.** `invoke(...)` from `index.html`, state loaded in
   `refreshAll`.
7. **CLI.** Command in `cli/src/args.rs` and `main.rs`, and a line in
   `cli/tests/smoke.sh`.
8. **Tests.** Unit-test the logic as a pure function. Add an endpoint test
   in `ui/tests/api.spec.mjs`; **CI fails if a Tauri command has no test**,
   is not registered, or has no matching D-Bus method.
9. **Quickshell**, if it should appear there. **Docs**: `docs/dbus.md`,
   `docs/cli.md`.

### Conventions

- Comments explain why, often with the incident behind the code. Keep that.
- A `ponytail:` comment marks a deliberate simplification and names its
  limit and upgrade path. Search for them before optimising.
- The frontend stays one file with no build step.
- Prefer the smallest change that works; no speculative abstractions.

## Known gaps

- No handling for PipeWire restarting while the daemon runs.
- Tests check stored state and call results, never what the audio sounds
  like. The fake test apps are silent.
- `pw.rs` (outside its pure helpers) and the D-Bus layer have no unit
  tests; they are covered by the endpoint tests only.
- No UI tests for the Outputs tab, cue, pan, fader groups or keyboard
  control. No Quickshell tests. No lint or format check in CI.
- Mute, solo, fader links and fader groups are Tauri-only client state.
- `buffer_ms` applies only to routes created after the change.
- A browser's tabs share one app identity, so the last manual move wins for
  all of them unless a rule matches by site.
- No release, Flatpak manifest untested.

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| No audio from a routed app | Check `pw-link -l`; its links should go to the group's `playback_FL` / `playback_FR` |
| Audio heard twice, phasing | The stream is linked to a group and directly to hardware |
| UI shows "Reconnecting" | Daemon is not answering on the session bus |
| Daemon restarts every few seconds | Watchdog firing on a stuck engine thread, or a startup failure; read `journalctl --user -u sonicforge-daemon` |
| Daemon killed with SIGKILL and no log | rtkit set `RLIMIT_RTTIME` to 0 because of a stale `xdg-desktop-portal`; restart the portal |
| Default group pinned near 0 dBFS, feeding itself | All hardware sinks vanished and a `sonicforge.*` sink became the system default; WirePlumber may need a restart |
| Underruns climbing | Doctor tab reports the rate; raise the buffer size in Settings |

Useful inspection commands: `wpctl status`, `pw-link -l`, `pw-top`,
`pw-dump`, `sonicforge system status`, `sonicforge system doctor`.

## Where to read more

| Topic | File |
|---|---|
| Processes, threads, state | `docs/architecture.md` |
| PipeWire nodes, routing, ring buffer, outputs | `docs/audio-engine.md` |
| DSP stage by stage | `docs/dsp.md` |
| Every D-Bus method and signal | `docs/dbus.md` |
| Every CLI command | `docs/cli.md` |
| Tauri app, tabs, client-side state | `docs/ui.md` |
| Quickshell surfaces | `docs/quickshell.md` |
| Test suites and the incident history behind the engine | `docs/testing.md` |
| Why things are the way they are | `docs/decisions.md` |
| Install script and systemd units | `docs/packaging.md` |
| Open work | `docs/roadmap.md` |
