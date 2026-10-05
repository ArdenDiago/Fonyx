# Development

A practical guide to working on SonicForge.

## Prerequisites

- Linux with a running PipeWire session and WirePlumber (`wpctl`)
- Rust and Cargo
- `clang`, for `pipewire-rs`'s bindgen step
- `webkit2gtk-4.1`, for Tauri
- `busctl` (part of systemd), for poking the daemon and for Quickshell
- Optional: `quickshell`; Node.js and npm for the UI tests

On Arch Linux:

```bash
pacman -S clang webkit2gtk-4.1
```

The `pipewire` crate is pinned to 0.10 in `daemon/Cargo.toml`. On the
development system (PipeWire headers 1.6.8) the 0.8 bindings did not build
correctly.

## Layout

| Path | What you edit there |
|---|---|
| `daemon/src/pw.rs` | PipeWire engine and audio callbacks |
| `daemon/src/dbus.rs` | D-Bus methods |
| `daemon/src/dsp/` | DSP building blocks |
| `daemon/src/*.rs` | Feature modules: scenes, rules, ducking, recording, plugins, doctor, graph |
| `ui/index.html` | The whole frontend |
| `ui/src-tauri/src/` | Tauri backend: `daemon.rs` (proxy), `commands.rs`, `settings.rs` |
| `ui/tests/` | Endpoint and UI tests, the D-Bus bridge, the isolated runner |
| `.github/workflows/` | CI |
| `quickshell/` | QML surfaces |
| `packaging/` | Install script and systemd units |

## Build

```bash
cargo build --workspace                 # debug build of daemon and UI
cargo build --release -p mtxpanel-daemon -p mtxpanel-ui
cargo clippy --workspace
```

## Run

```bash
RUST_LOG=info cargo run -p mtxpanel-daemon
cargo run -p mtxpanel-ui                # second terminal
```

The daemon claims the bus name `org.mtxpanel.Daemon`. Only one instance can
hold it. If the daemon is installed as a user service, stop it before
running a development build:

```bash
systemctl --user stop mtxpanel-daemon
```

A development daemon reads and writes the same
`~/.config/mtxpanel/config.toml` as an installed one. To experiment without
touching your real config, point it elsewhere:

```bash
XDG_CONFIG_HOME=/tmp/mtx-dev cargo run -p mtxpanel-daemon
```

Starting or stopping the daemon re-links every application's audio, which
causes a brief interruption.

Quickshell surfaces:

```bash
quickshell -p quickshell/MtxPanelApp.qml
quickshell -p quickshell/MtxPanelPopup.qml
```

## Test

```bash
cargo test -p mtxpanel-daemon           # unit tests, no PipeWire needed

cd ui/tests && npm ci && npx playwright install chromium && cd ../..   # once
cargo build -p mtxpanel-daemon
sh ui/tests/run-isolated.sh             # endpoint + UI tests in a throwaway session
```

`run-isolated.sh` starts its own PipeWire, D-Bus session and daemon, so it
does not touch your real audio or config. It needs `pipewire`,
`wireplumber` and `dbus-run-session`. See [Testing](testing.md).

## Contributing changes

`main` is protected. Work on a branch, push it, and open a pull request:

```bash
git checkout -b my-change
git push -u origin my-change
gh pr create --fill
```

CI builds the workspace and runs every test on the pull request. The
"Build and test" check has to pass before the pull request can be merged.
Run `cargo test --workspace` and `sh ui/tests/run-isolated.sh` locally
first to get the same result sooner.

## Driving the daemon by hand

```bash
D="org.mtxpanel.Daemon /org/mtxpanel/Daemon org.mtxpanel.Daemon"

busctl --user introspect org.mtxpanel.Daemon /org/mtxpanel/Daemon
busctl --user call $D ListGroups
busctl --user call $D ListStreams
busctl --user --json=short call $D GetLevels
busctl --user call $D RunHealthChecks
```

See [D-Bus](dbus.md) for the method list.

## Inspecting the graph

| Command | Shows |
|---|---|
| `wpctl status` | Sinks (including group sinks), streams, the default sink |
| `pw-link -l` | Every link; use it to confirm where a stream is connected |
| `pw-link -l -I` | The same with port ids |
| `pw-top` | Per-node scheduling and error counts |
| `pw-dump` | Full object properties, for example port channel labels |
| `journalctl --user -u mtxpanel-daemon -f` | Logs of the installed daemon |

Group sinks are named `mtxpanel.group.<id>`, route playback streams
`mtxpanel.<label>.route.<output or default>`, and meter taps
`mtxpanel.meter.<stream id>`.

## Adding a feature end to end

A new control usually touches these places, in this order. The EQ level line
and pan are small, complete examples to follow.

1. **Model.** Add a field to `Group` (`daemon/src/group.rs`) or `Config`
   with `#[serde(default)]`, so existing config files still load.
2. **Engine.** Add a `Command` variant in `pw.rs`, a live parameter
   (`Smoothed` for anything read per sample) and its use in the callback.
   Apply it in `activate_scene` if scenes should carry it.
3. **D-Bus.** Add a setter in `dbus.rs` that updates `Config`, saves, sends
   the command and emits a signal. Add a getter or extend a list method.
4. **Tauri.** Add the method to the proxy trait in `daemon.rs`, a command in
   `commands.rs`, and register it in `lib.rs`.
5. **Frontend.** Call it with `invoke(...)` from `index.html` and load its
   state in `refreshAll`.
6. **Tests.** Put the logic in a pure function and unit-test it. Add an
   endpoint test for the new command in `ui/tests/api.spec.mjs`; CI fails
   if a command has none. Add a smoke test if it has a UI path.
7. **Quickshell.** Add it there too if it should appear in those surfaces.

Avoid changing the tuple shape of an existing D-Bus method. Three clients
read those tuples by position. Add a new method instead, as `ListGroupPans`
and `ListStreamSides` do.

## Rules for audio-callback code

- Do not allocate, log, block or do I/O in a `process` callback.
- Read per-sample values from atomics. Use `Smoothed` for anything the user
  can change while audio is playing.
- Callbacks run on PipeWire's realtime thread. Never share state with them
  through `Rc<RefCell<_>>`.
- Do not hold the config lock across a `wpctl` call or any other blocking
  operation.
- Recompute filter coefficients per buffer, not per sample, and never reset
  filter history on a parameter change.

## Conventions

- Source comments explain why, often with the incident that led to the
  code. Keep that habit for non-obvious behaviour.
- A `ponytail:` comment marks a deliberate simplification and names its
  limit and upgrade path. Search for them before optimising.
- Structured D-Bus data is positional tuples.
- The frontend is one file with no build step. Keep it that way unless there
  is a strong reason.

## Debugging notes

- **No audio from a routed app.** Check `pw-link -l` for the stream's
  links. They should go to its group's `playback_FL` / `playback_FR` ports.
- **Audio heard twice or phasing.** A stream is linked both to a group and
  directly to hardware.
- **UI shows "Reconnecting".** The daemon is not answering on the session
  bus; check `systemctl --user status mtxpanel-daemon` or the terminal
  running it.
- **Daemon restarts every few seconds under systemd.** The watchdog is
  firing because the engine thread is stuck, or the daemon is failing at
  startup; read the journal.
- **Underruns climbing.** Open the Doctor tab, which reports the recent
  rate, and consider a larger buffer size in Settings.
