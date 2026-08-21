# MTX Panel

Per-application audio mixer/router for Linux (PipeWire). See `PROJECT_SPEC.md`
for the full brief this scaffold follows.

## Layout

```
daemon/            Rust daemon — owns PipeWire + DSP, runs as systemd --user unit
  src/pw.rs           PipeWire engine: connects, tracks app streams, creates a
                       virtual Audio/Sink node per group, links routed streams
                       into it port-by-port, runs the full DSP chain in its
                       process callback (stages 1-3, 5, 6 — done and live-tested)
  src/group.rs        Group data model (persisted)
  src/config.rs        TOML config persistence (~/.config/mtxpanel/config.toml)
  src/dbus.rs          org.mtxpanel.Daemon D-Bus interface (stage 4 — done,
                       wired into main, updates both Config and the live engine)
  src/dsp/smoothed.rs    Lock-free ramped parameter (gain, EQ, comp/gate — stage 3)
  src/dsp/biquad.rs      RBJ peaking/highpass/lowpass biquads (stage 5)
  src/dsp/dynamics.rs    Compressor + gate, feed-forward gain-computer (stage 6)

ui/                 Tauri app (Home, Inputs, MTX Panel, Faders, Settings — stage 7)
  src-tauri/           Rust backend: a thin D-Bus client (daemon.rs) exposing
                       one Tauri command per D-Bus method (commands.rs)
  src-tauri/icons/      Placeholder app icon (solid color) — swap for real branding
  index.html            Live UI: all 5 tabs, vanilla JS, no framework — talks to
                       the daemon via window.__TAURI__.core.invoke

quickshell/         Quickshell QML quick-access widget (stage 8 — still a stub;
                    Quickshell's D-Bus QML API wasn't verified against real
                    docs the way everything else here was, see the file)

packaging/
  flatpak/             Flatpak manifest (stage 9 — unverified, last in the build order)
  systemd/             mtxpanel-daemon.service user unit
```

Build order follows `PROJECT_SPEC.md` Section 9. **Stages 1-7 are implemented
and live-tested** (daemon built, run, and driven over real D-Bus calls against
a real PipeWire session with real app audio streams; UI built, run, and
confirmed rendering live daemon data). Stage 8 (Quickshell) is a stub; stage 9
(Flatpak) is unverified.

## What actually works, verified live (not just "it compiles")

- Daemon connects to PipeWire, enumerates app audio streams, creates a
  `media.class=Audio/Sink` virtual node per group (shows up as a real
  selectable sink in `wpctl status`).
- `RouteStreamToGroup` links an app stream's ports into the group's sink
  port-by-port (channel-matched) — confirmed via `pw-link -l`.
- The full per-group chain — gain → EQ (up to 6 bands) → HPF/LPF → compressor
  → gate, all parameter-smoothed — runs in the capture stream's process
  callback and hands off to the playback stream via a lock-free ring buffer.
  Confirmed via `pw-top`: both stream nodes run at `ERR 0` (no xruns) with a
  live EQ band and gain change applied mid-stream.
- `org.mtxpanel.Daemon` is live on the session bus; every method both updates
  `~/.config/mtxpanel/config.toml` and forwards to the running engine.
  Confirmed with `busctl --user call`.
- The Tauri UI's Home tab, driven purely through Tauri commands →
  `zbus::blocking` → D-Bus, renders the real group list from a running
  daemon (screenshotted).

## Known simplifications (see `ponytail:` comments in the code for more)

- **Single mainloop thread**: everything in `pw.rs` — registry events,
  D-Bus-triggered commands, and every stream's audio callback — runs on one
  thread (pipewire-rs's `pw_main_loop`, not `pw_thread_loop`). An infrequent
  control op could in principle delay another group's audio by a sliver.
  Fine for "a handful of apps" desktop use; upgrade to `pw::thread_loop` if
  xruns ever show up in practice.
- **Fixed audio format**: every group's streams are hardcoded to 48kHz
  stereo F32LE rather than negotiating the graph's native rate — this is the
  resolution of Section 11's "default sample rate handling" open question.
- **Master fader precision**: with N groups active, the master gain ramp
  (Section 7.4) ticks N times per real sample since each group's callback
  advances the same shared ramp — converges faster than configured, not
  audibly broken, documented at the call site in `pw.rs`.
- Routing a stream into a group does **not** sever its original direct
  connection to hardware — both play simultaneously until that's addressed.

## Building

```
cargo build --workspace
cargo test -p mtxpanel-daemon    # smoothing/biquad/dynamics self-checks
```

Needs `clang` (pipewire-rs's bindgen step) and `webkit2gtk-4.1` (Tauri)
installed as system packages — on Arch, `pacman -S clang webkit2gtk-4.1`.

Note: `pipewire = "0.10"` in `daemon/Cargo.toml`, not `0.8` — this system's
pipewire/spa headers (1.6.8) aren't ABI-compatible with the 0.8.x Rust
bindings (`spa_pod_builder`'s fields come back opaque under bindgen against
that header version); 0.10.1 builds clean against it.

## Running

```
cargo run -p mtxpanel-daemon    # RUST_LOG=info for stream/group logging
cargo run -p mtxpanel-ui        # separate terminal, needs the daemon running
```

Or drive the daemon directly without the UI, e.g.:

```
busctl --user call org.mtxpanel.Daemon /org/mtxpanel/Daemon org.mtxpanel.Daemon CreateGroup s "Music"
busctl --user call org.mtxpanel.Daemon /org/mtxpanel/Daemon org.mtxpanel.Daemon RouteStreamToGroup uu <stream_id> <group_id>
```

## Decisions made on OPEN items (Section 11)

- **Config format/location**: TOML at `~/.config/mtxpanel/config.toml`
  (`daemon/src/config.rs`).
- **Default sample rate handling**: fixed at 48kHz stereo F32LE for every
  group (see "Known simplifications" above) rather than per-app native rates.

## Still open

- Exact semantics of "global fader configuration" (Section 7.4) — link
  faders together vs. one shared offset. The unambiguous master fader from
  the same section *is* built; this separate "global mode" bullet is not.
- Full Settings tab option list (Section 7.5) — the tab is a placeholder.
- Quickshell widget's actual D-Bus QML API (stage 8) — flagged in
  `quickshell/MtxPanelWidget.qml` rather than guessed at.
