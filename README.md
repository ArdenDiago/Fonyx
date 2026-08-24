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
                       one Tauri command per D-Bus method (commands.rs), plus
                       settings.rs for the one bit of non-D-Bus backend state
                       (autostart, via systemctl --user)
  src-tauri/icons/      Placeholder app icon (solid color) — swap for real branding
  index.html            Live UI: all 5 tabs, vanilla JS, no framework — talks to
                       the daemon via window.__TAURI__.core.invoke. EQ/HPF/LPF
                       curve is a real RBJ-biquad magnitude-response plot (not
                       decorative), hydrated from the daemon's actual persisted
                       state on load/group-switch rather than assumed zeros

quickshell/         Quickshell QML quick-access widget (stage 8 — wired up).
                    Quickshell has no generic "call any D-Bus method" QML type
                    (checked against quickshell.org's docs — only protocol-
                    specific wrappers like Mpris/UPower/SystemTray exist), so
                    this shells out to `busctl --json=short` via Quickshell.Io's
                    Process, the same mechanism the busctl examples below use.
                    Live-tested: ran under the real `quickshell` binary against
                    a running daemon and confirmed it parses a real ListGroups
                    reply into real group data.

packaging/
  flatpak/             Flatpak manifest (stage 9). Runtime switched to
                       org.gnome.Platform (ships webkit2gtk-4.1, Tauri's
                       webview dep, per Tauri's own Flatpak docs) instead of
                       bare org.freedesktop.Platform. cargo-sources.json is a
                       real generated offline vendor manifest (1011 crates,
                       from this repo's actual Cargo.lock via flatpak-builder-
                       tools' flatpak-cargo-generator.py), not a placeholder.
                       Desktop file + AppStream metainfo added and validated
                       with `appstreamcli validate` (clean but for a homepage-
                       URL warning — no public repo URL exists yet). Not run
                       through an actual `flatpak-builder` build (not
                       installed in the environment this was built in) — the
                       first real build is still the real test.
  systemd/             mtxpanel-daemon.service user unit
```

Build order follows `PROJECT_SPEC.md` Section 9. **Stages 1-8 are implemented
and live-tested** (daemon built, run, and driven over real D-Bus calls against
a real PipeWire session with real app audio streams — including routing a
real generated tone through a group's full gain/EQ chain; UI built, run
headless against the live daemon and confirmed rendering real data on every
tab; Quickshell widget run under real `quickshell` against the live daemon).
Stage 9 (Flatpak) is manifest-complete but unverified by an actual build.

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
- **Home tab metrics are real**: `sysinfo`-backed CPU/RAM, plus the daemon's
  own negotiated buffer size and a ring-underrun counter, exposed over a new
  `GetMetrics` D-Bus method and polled every 200ms. Live-tested: idle groups
  correctly read 0 underruns (see the fix note below); routing a real
  generated tone through a group and reading `GetLevels` showed real
  nonzero peak levels tracking the signal.
- **EQ/HPF/LPF curve, group state hydration, and compressor gain-reduction
  meter are real**, not decorative: `GetGroupDetails` (new) returns a
  group's actual persisted EQ bands/filters/compressor/gate, so the MTX
  Panel shows what the daemon is actually running rather than assumed
  zeros; `GetLevels` also carries live compressor gain reduction for the
  MTX Panel's GR meter. Live-tested against a group with a real EQ band
  already enabled from a prior session — the UI correctly rendered its
  real +6dB/1kHz curve instead of a flat line.
- **Fader linking** (Section 7.4's "global" open item, resolved): a 🔗
  toggle per fader groups it into a linked set; moving one applies the same
  delta to the others, DAW-group-fader style. Client-side only (localStorage
  for which faders are linked) — no daemon change needed since it's just
  calling `SetGain` per linked group.
- **Quickshell widget** talks to the daemon for real via `busctl
  --json=short` (see the `quickshell/` note above) — no daemon or D-Bus
  interface changes needed for this.
- **Fixed: routing a stream into a group used to leave its original direct
  connection to hardware alive too** (previously listed below as a known
  gap) — reported by a real user as audible "oscillating"/phasing, which
  makes sense: the same signal reaching hardware twice, once raw and once
  through the group's processing + ring-buffer latency, is exactly a comb
  filter. `pw.rs` now tracks every `Link` global (`Engine.links`) and
  destroys a stream's existing links before creating a new one
  (`Engine::sever_existing_links`); a new `UnrouteStream` D-Bus method does
  the same in reverse, reconnecting the stream to whatever hardware sink a
  group's own (always-autoconnected) playback stream is feeding. Live-
  verified with `pw-link -l` before/after: routing severed the direct
  hardware link and left only the group link; unrouting severed the group
  link and recreated the hardware one.
- **Inputs tab now shows which stream is actually making sound, and a
  routed stream disappears from Inputs into its group.** Most apps
  (Firefox included — checked live, no `media.name` on its stream nodes)
  don't expose a per-tab title over PipeWire, so multiple tabs previously
  all showed as identical, indistinguishable "Firefox" rows. `AppStream`
  now tracks each node's live `RUNNING` state (bound via a `Node` proxy +
  info listener in `pw.rs`), shown as a lit dot; the UI disambiguates
  same-named streams as "Firefox #1"/"Firefox #2" and sorts playing ones
  first. `ListGroups` now also returns each group's `routed_streams`, so
  the Inputs list filters out anything already routed (real, persisted
  server state — not a client-side-only "for this session" list like
  before) and the Groups panel lists them nested underneath with a ✕ that
  calls the new `UnrouteStream`.
- **Full visual redesign**, per explicit direction to match FabFilter
  Pro-Q's UI language (PROJECT_SPEC.md Section 7.3 already named this
  reference for the EQ specifically): dark token system across every tab
  (Inter for labels, JetBrains Mono for numeric readouts), replacing the
  earlier flat pastel/Figma-mock look. The MTX Panel's EQ is no longer
  separate knobs + a static curve + a scrollable band list — it's one
  interactive graph: draggable circular nodes per band (drag = freq/gain,
  scroll = Q), colored per band, with HPF/LPF as their own draggable nodes
  pinned to the 0dB line. Live-verified end-to-end, not just visually: a
  simulated real pointer drag on a graph node in a headless browser against
  a running daemon changed the node's on-screen position *and* the
  daemon's persisted `GetGroupDetails` frequency to match.

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
- **The capture→playback ring buffer underruns measurably during real
  playback, and this needs investigation.** Routing a real generated tone
  through a group and watching the new `GetMetrics` underrun counter showed
  it climbing at roughly a third of the sample rate — not the idle-group
  false-positive this session found and fixed (see `GroupLive::has_input`'s
  doc comment in `pw.rs`), but a real, sustained rate during active
  playback. `pw-top`'s own `ERR` column stayed at 0 the entire time (the
  PipeWire graph driver itself reports clean, on-time callbacks from both
  streams), so this looks like it's specifically the hand-rolled
  capture→playback ring buffer running dry, not an audible graph-level
  xrun — but that's an inference, not confirmed by ear or a proper
  profiler. Filed here rather than silently shipped, since it contradicts
  this file's earlier ("pre-metrics") claim of confirming glitch-free
  operation via `pw-top` alone; `pw-top`'s `ERR` column and "is the audio
  actually clean" turn out not to be the same question. Investigate with
  real playback + a profiler listener before trusting the underrun count
  as a proxy for audible quality, and don't be surprised if it's nonzero
  during normal use in the meantime.

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
- **"Global fader configuration" (Section 7.4)**: resolved as fader
  *linking* (🔗 groups faders so moving one applies the same delta to the
  rest), not a single shared gain offset — the master fader already covers
  the "one offset over everything" case, so linking is the reading that
  adds something distinct. Flagging the decision here per this doc's own
  Section 1 rule, rather than blocking on it.
- **The ring-underrun metric counts our own capture→playback ring
  starving, not the PipeWire graph's own driver xrun count** — see "Known
  simplifications" above for why, and for a real issue that surfaced while
  building it.

## Still open

- Full Settings tab option list (Section 7.5) — autostart is real (toggles
  the systemd --user unit); theme, device selection, and buffer size are
  still just placeholders/read-only.
- The ring-buffer underrun-during-playback finding above.
- Flatpak manifest is unverified by an actual `flatpak-builder` run.
