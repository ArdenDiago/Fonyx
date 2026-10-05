# Quickshell

The repository ships three Quickshell (QML) surfaces. All are thin clients
of the daemon's D-Bus interface, with no DSP or routing logic of their own.

| File | What it is |
|---|---|
| `quickshell/MtxPanelWidget.qml` | A content `Item` with no window: group faders, mute, solo, output routing and master gain. Meant to be embedded in a status-bar popup. |
| `quickshell/MtxPanelPopup.qml` | The widget inside its own `FloatingWindow`, for a compositor keybind. |
| `quickshell/MtxPanelApp.qml` + `quickshell/app/` | A standalone rebuild of the desktop UI in QML, in its own `FloatingWindow`. |

## How it talks to the daemon

Quickshell has no generic QML type for calling an arbitrary D-Bus method.
Its D-Bus support is a set of protocol-specific wrappers (DBusMenu, MPRIS,
UPower, system tray and similar), which do not cover a custom interface such
as `org.mtxpanel.Daemon`.

The surfaces therefore use `Quickshell.Io`'s `Process` type to run `busctl`:

```text
busctl --user --json=short call \
    org.mtxpanel.Daemon /org/mtxpanel/Daemon org.mtxpanel.Daemon <Method> [signature args…]
```

and parse the JSON it prints. `--json=short` wraps the reply as
`{"type": "...", "data": [...]}`, where `data` has one entry per D-Bus out
argument. Replies are used as positional arrays in the same field order the
daemon defines.

- The widget builds its command lines with a `busctlCall` helper and keeps
  one `Process` per call it makes.
- The app uses `app/DbusCall.qml`, a reusable `Process` created per call
  with a method, signature, arguments and result callback, which destroys
  itself when the reply has been read. This avoids a hand-written `Process`
  for each of the roughly fifty methods the app uses.

This needs no change to the daemon and no extra D-Bus interface. Its cost is
a process spawn per call.

## Polling

There is no signal subscription. Each surface polls:

| Surface | Interval | What |
|---|---|---|
| App | 5 s | Groups, streams, outputs |
| App, MTX tab | 200 ms | Levels for the selected group |
| App, Home tab | 500 ms | Levels and metrics |
| Widget | A configurable interval | Groups, outputs, master gain, and each group's routes |

The Home tab polls more slowly than the Tauri UI because a `busctl` spawn is
far heavier than an in-process D-Bus call, and a tighter interval would
stack overlapping calls.

## The standalone app

`MtxPanelApp.qml` holds the shared state (groups, streams, outputs) and
passes it to each tab as plain arrays. The tabs are in `quickshell/app/tabs/`:

`HomeTab`, `InputsTab`, `MtxTab`, `FadersTab`, `OutputsTab`, `ScenesTab`,
`RulesTab`, `RecordingTab`, `DoctorTab`, `SettingsTab`.

Shared components are in `quickshell/app/`: `EqGraph`, `CompGraph`,
`GateDoor`, `SpectrumCanvas`, `SignalGraph`, `Knob`, `Slider2`, `DbScale`,
`GroupCombo`, `OutputCombo`, `Theme` and `DbMath.js`.

## The widget

The widget's scope is deliberately narrow: per-group faders, mute, solo,
output routing and master gain. EQ, dynamics, scenes, rules, recording,
Doctor and settings stay in the full applications.

It has no window. A status bar that this project does not provide is
expected to anchor it. `MtxPanelPopup.qml` wraps it in a window for use
without a bar, toggled from a compositor keybind:

```bash
pkill -f MtxPanelPopup.qml || quickshell -p ~/.local/share/mtxpanel/quickshell/MtxPanelPopup.qml
```

## Running

During development, from the repository root:

```bash
quickshell -p quickshell/MtxPanelApp.qml
quickshell -p quickshell/MtxPanelPopup.qml
```

The daemon must be running.

## Autostart

`packaging/systemd/mtxpanel-quickshell.service` starts
`MtxPanelApp.qml` with the graphical session. It targets the app, not the
widget, because the widget has no window of its own.

The Settings tab's "Quickshell autostart" switch enables or disables this
unit with `systemctl --user`. The unit must be installed first;
`packaging/install-user.sh` installs it without enabling it. See
[Packaging](packaging.md).

## Limitations

- **Feature lag.** The QML app was written to match the Tauri UI at one
  point in time. Controls added to the Tauri UI since then are not all
  present; for example, the QML code has no EQ level line, pan, mono/stereo
  or per-stream left/right controls.
- **No shared client state.** Mute, solo, links and fader groups in the
  Tauri UI are stored in its webview and are not visible here.
- **Spawn cost.** Every call starts a `busctl` process.
- **Tested on one setup.** The surfaces have only been run on the
  developer's own Quickshell and compositor configuration.
