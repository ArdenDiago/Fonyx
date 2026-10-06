# Desktop UI

The desktop application is `sonicforge-ui`, a Tauri 2 app. It is a thin
client: it holds no audio or routing logic and can be closed, restarted or
left out entirely without affecting audio.

## Structure

```text
ui/
├── index.html          the entire frontend: markup, CSS and JavaScript
├── src-tauri/
│   ├── src/lib.rs      Tauri setup, command registration
│   ├── src/daemon.rs   zbus proxy for org.sonicforge.Daemon
│   ├── src/commands.rs one Tauri command per D-Bus method
│   ├── src/settings.rs autostart toggles via systemctl
│   ├── tauri.conf.json
│   └── icons/icon.png  placeholder icon
└── tests/              Playwright smoke tests (see testing.md)
```

There is no frontend build step, bundler or framework. Tauri serves
`ui/index.html` directly (`frontendDist: "../"`) and exposes its API on
`window.__TAURI__` (`withGlobalTauri: true`).

## Call path

```text
Frontend (index.html)
   │  invoke('set_gain', { groupId, value })
   ▼
Tauri command (commands.rs)
   │  state.0.set_gain(group_id, value)
   ▼
zbus blocking proxy (daemon.rs)
   │  D-Bus method call, 3 s timeout
   ▼
sonicforge-daemon
```

- `daemon.rs` declares the D-Bus interface as a `zbus` proxy trait.
- `commands.rs` wraps each proxy method in a `#[tauri::command(async)]`
  function with the same name. There are about 80 of them, and they contain
  no logic beyond passing arguments through and converting errors to
  strings.
- The frontend uses snake_case command names with camelCase argument names,
  which Tauri maps to the Rust parameters.
- The connection is made once at startup. The UI therefore needs the daemon
  to be running when it starts.
- Each call has a 3-second timeout, so a stuck daemon fails one call rather
  than blocking a worker.

`settings.rs` is the only backend code that does not talk to the daemon. Its
four commands enable or disable the `sonicforge-daemon.service` and
`sonicforge-quickshell.service` user units with `systemctl --user`.

## State handling

The daemon is the source of truth. The frontend does not subscribe to D-Bus
signals; it polls:

| Loop | Interval | Calls |
|---|---|---|
| `refreshAll` | 5 s | Groups, streams, feeds, pans, stream sides, outputs, scenes, rules, ducking, recordings, health |
| `pollLive` | 200 ms | Levels, spectrum, metrics, stream levels |

Controls are populated from the daemon's actual state on load and on group
switch (`GetGroupDetails`), not from assumed defaults.

If a call fails, the status indicator switches to "Reconnecting"; the next
successful call restores it and triggers a full refresh.

### Client-side state

Some state exists only in the webview's local storage, because it is
implemented in the UI on top of the daemon's primitives:

| Feature | How it works |
|---|---|
| Group mute and solo | The UI keeps the intended fader value and sends gain 0 for muted groups |
| Fader links | Moving a linked fader applies the same change to the others, as separate `SetGain` calls |
| Fader groups | A named set of groups with its own fader that scales its members; each has its own colour |
| Stream mute and solo | Applied with `SetStreamMuted`; solo with cue outputs moves the stream to the Cue group |
| Custom EQ presets | Saved band settings |
| Theme | Light or dark |

Because this state is local to the Tauri webview, the Quickshell surfaces do
not share it.

## Tabs

| Tab | Contents |
|---|---|
| **Home** | Master level meters and fader, input spectrum, CPU/RAM/buffer/underrun tiles, device overview |
| **Inputs** | Application streams with live level, mute, solo, left/right/stereo and Own DSP; groups with their streams; fader groups |
| **MTX Panel** | For the selected group: interactive EQ graph with HPF/LPF, compressor and gate sub-tabs, plugin chain |
| **Faders** | One fader per group with link, mute, solo, mono/stereo and pan; fader-group strips; master fader |
| **Outputs** | One card per device: volume, mute, cue marking, and each group's or Master's route with gain and sync |
| **Scenes** | Capture, activate, update, duplicate and delete scenes |
| **Rules** | Rule builder and history; ducking rules; cue duck level |
| **Recording** | Start and stop recordings of a group |
| **Doctor** | Health checks and the signal-flow graph |
| **Settings** | Daemon autostart, Quickshell autostart, theme, preferred output device, buffer size |

## MTX Panel tab details

**EQ graph.** One SVG graph shows the combined response and a node per band,
plus high-pass and low-pass nodes on the 0 dB line. Dragging a node sets
frequency and gain, scrolling sets Q, and a horizontal level line moves the
whole curve. The curve is computed in JavaScript from the same RBJ biquad
formulas the daemon uses, so it shows the response the daemon applies. The
group's live input spectrum is drawn behind it.

Presets (Flat, Rock, Pop, Jazz, Movie, Bass Boost, Vocal Boost) and saved
custom presets set the bands in one step.

**Compressor.** A transfer curve (input dB against output dB) with a live
marker. The marker's output position is the measured peak, and its input
position is reconstructed as output minus the measured gain reduction. It
changes colour while compression is active.

**Gate.** A graphic that follows the gate's measured attenuation.

**Controls** are rotary knobs driven by vertical drag.

## Keyboard

The app is usable from the keyboard: single keys switch tabs (`H`, `I`, `E`,
`F`, `O`, `Shift+S`), and each tab has its own keys for selection, mute,
solo, fader moves and pickers. Press `/` in the app for the full list.

## Visual design

Dark and light themes share one set of CSS custom properties. Inter is used
for labels and JetBrains Mono for numeric readouts; both are loaded from
Google Fonts, so they fall back to system fonts offline. The EQ's
graph-first layout follows the FabFilter reference named in the project
specification.

## Limitations

- The app icon is a placeholder.
- The UI connects to the session bus once at startup and is intended to be
  started after the daemon.
- Client-side state (mute, solo, links, fader groups) is not visible to
  other clients and is lost if the webview's storage is cleared.
- Polling every 5 seconds means a change made by another client can take
  that long to appear.
- Content Security Policy is disabled in `tauri.conf.json`.
