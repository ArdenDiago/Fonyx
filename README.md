# MTX Panel

Per-application audio mixer/router for Linux (PipeWire). See `PROJECT_SPEC.md`
for the full brief this scaffold follows.

## Layout

```
daemon/            Rust daemon — owns PipeWire + DSP, runs as systemd --user unit
  src/pw.rs           PipeWire connection, app-stream enumeration (stage 1, done)
  src/group.rs        Group data model
  src/config.rs        TOML config persistence (~/.config/mtxpanel/config.toml)
  src/dbus.rs          org.mtxpanel.Daemon D-Bus interface (stage 4, not wired into main yet)
  src/dsp/gain.rs       Smoothed gain — reference impl of the click-free ramp pattern (stage 3)
  src/dsp/biquad.rs     RBJ peaking-EQ biquad (stage 5, math done, not wired into an audio callback)
  src/dsp/dynamics.rs   Compressor/gate param structs (stage 6, processing not implemented)

ui/                 Tauri app (Home, Inputs, MTX Panel, Faders, Settings — stage 7)
  src-tauri/           Rust backend, thin D-Bus client
  src-tauri/icons/      Placeholder app icon (solid color) — Tauri requires one to build, swap for real branding
  index.html            Placeholder frontend shell, no framework wired in yet

quickshell/         Quickshell QML quick-access widget (stage 8, thin D-Bus client)

packaging/
  flatpak/             Flatpak manifest (stage 9)
  systemd/             mtxpanel-daemon.service user unit
```

Build order follows `PROJECT_SPEC.md` Section 9. Current state: stage 1 is
implemented (daemon connects to PipeWire and logs each app stream it sees).
Everything past that is data model / interface shape only — no live
PipeWire routing, no audio callback, no DSP processing loop yet.

## Building

```
cargo build -p mtxpanel-daemon   # or --workspace for the Tauri UI too
cargo test -p mtxpanel-daemon    # gain-ramp + biquad self-checks
```

Verified working: both crates build clean, the daemon's unit tests pass, and
running the daemon binary against a live PipeWire session correctly logs
real app streams. Needs `clang` (pipewire-rs's bindgen step) and
`webkit2gtk-4.1` (Tauri) installed as system packages — on Arch,
`pacman -S clang webkit2gtk-4.1`.

Note: `pipewire = "0.10"` in `daemon/Cargo.toml`, not `0.8` — this system's
pipewire/spa headers (1.6.8) aren't ABI-compatible with the 0.8.x Rust
bindings (`spa_pod_builder`'s fields come back opaque under bindgen against
that header version); 0.10.1 builds clean against it.

## Decisions made on OPEN items (Section 11)

- **Config format/location**: TOML at `~/.config/mtxpanel/config.toml`
  (`daemon/src/config.rs`). Plain and human-editable, no DB dependency.

## Still open (flagged in the spec, not decided here)

- Exact semantics of "global fader configuration" (Section 7.4) — link
  faders together vs. one shared offset. Needs a decision before Faders tab
  work starts.
- Full Settings tab option list (Section 7.5).
- Default buffer size / sample-rate handling when app streams request
  different rates — relevant once stage 2 (group nodes + routing) starts.
