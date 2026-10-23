# Packaging

Fonyx has no published packages and no tagged release. It is installed
from a source checkout with the files in `packaging/`:

| Path | What it is |
|---|---|
| `install-user.sh` | Per-user install from a source checkout |
| `systemd/` | Three systemd user units, used by the install script |
| `branding/` | The logo artwork and the script that cuts the icons out of it; the Snap Store banner and its source |

## User install

```bash
git clone https://github.com/ArdenDiago/Fonyx.git
cd Fonyx
sh packaging/install-user.sh
```

The script needs no root. It:

1. Builds release binaries: `cargo build --release -p fonyx-daemon -p fonyx-ui -p fonyx-cli`.
2. Installs `fonyx-daemon`, `fonyx-ui` and the `fonyx` CLI
   ([Command line](cli.md)) into `~/.local/bin`. Each is
   copied under a temporary name and renamed, because overwriting a running
   binary in place fails.
3. Installs a launcher entry and the app icon under `~/.local/share`
   (`applications/org.fonyx.app.desktop` and
   `icons/hicolor/512x512/apps/org.fonyx.App.png`).
4. Writes the daemon and UI units into `~/.config/systemd/user/`, rewriting
   `ExecStart` from `/usr/bin/...` to `~/.local/bin/...`.
5. Copies `quickshell/` to `~/.local/share/fonyx/quickshell/` and writes
   the Quickshell unit pointing there.
6. Stops any hand-started daemon that would hold the D-Bus name.
7. Reloads systemd, enables the daemon and UI units, restarts the daemon,
   and restarts the UI and Quickshell units if they are running.

The Quickshell unit is installed but not enabled; the Settings tab toggles
it.

Re-run the script after pulling changes to install the new build. Restarting
the daemon interrupts audio briefly while streams are re-routed.

To remove the install, disable the units and delete the files the script
created:

```bash
systemctl --user disable --now fonyx-ui fonyx-quickshell fonyx-daemon
rm ~/.local/bin/fonyx-daemon ~/.local/bin/fonyx-ui ~/.local/bin/fonyx
rm ~/.config/systemd/user/fonyx-*.service
rm -r ~/.local/share/fonyx
rm ~/.local/share/applications/org.fonyx.app.desktop
rm ~/.local/share/icons/hicolor/512x512/apps/org.fonyx.App.png
systemctl --user daemon-reload
```

Configuration in `~/.config/fonyx/` is left in place.

## systemd units

All three are **user** units.

### `fonyx-daemon.service`

| Setting | Value | Reason |
|---|---|---|
| `Type` | `notify` | The daemon reports `READY=1` once PipeWire is connected |
| `WatchdogSec` | `10` | The daemon pings every 2 s from its engine thread; a hang is treated like a crash |
| `Restart` | `on-failure`, after 1 s | Recover from crashes and watchdog kills |
| `StartLimitBurst` | 5 in 60 s | Stop a crash loop |
| `After` / `Wants` | `pipewire.service` | Start after PipeWire |
| `WantedBy` | `default.target` | Start at login |

### `fonyx-ui.service`

Starts `fonyx-ui` with the graphical session, after the daemon.
`Restart=on-failure`.

### `fonyx-quickshell.service`

Runs `quickshell -p .../FonyxApp.qml` with the graphical session. It
starts the standalone app, not the widget, which has no window of its own.

The packaged units use `/usr/bin` and `/usr/share/fonyx` paths, as a
system package would. The install script rewrites them for a per-user
install.

### Autostart toggles

The Settings tab's two autostart switches call
`systemctl --user enable|disable` on the daemon and Quickshell units. They
work only once the unit files are installed.

## Snap Store listing

The listing at <https://snapcraft.io/fonyx> is filled in by hand on the
store's web form; nothing in the repository uploads it.

| Field | Value |
|---|---|
| Category | Music and Audio, then Utilities |
| Summary | Per-application audio mixer and router for PipeWire |
| Featured banner | `packaging/branding/banner.jpg` (2:1) or `banner-3x1.jpg` (3:1), whichever ratio the form's image restrictions ask for, uploaded as `banner.jpg` |
| Screenshots | `docs/images/ui-mtx.png`, `ui-doctor.png`, `ui-outputs.png`, `ui-home.png`. The banner counts toward the store's limit of five images. |

The banner is `packaging/branding/banner.html`, rendered by
`node packaging/branding/make-banner.mjs` (needs `npm ci` in `ui/tests`
and `site/`). The screenshots come from a throwaway PipeWire session with
four demo apps playing synthetic audio, captured at 1440x900 and 1.5x scale.

Description:

```text
Fonyx is a per-application audio mixer and router for Linux, built on PipeWire.

Put each application's audio in a group you define. Every group gets its own fader and DSP chain, and can be sent to one or more output devices at once.

Features:
* Per-group fader, high-pass and low-pass filters, 6-band EQ, compressor, gate and pan
* Built-in plugins: soft clipper, stereo widener, deep bass, mids
* Route any group to several outputs, each with its own gain and delay
* Master bus, sub-groups and a Cue group for soloing to headphones
* Scenes, profiles, rules and ducking that move audio automatically
* WAV/FLAC recording of groups
* Health checks (Doctor) and a live signal-flow graph
* Desktop app (fonyx.ui) and a full command-line tool (fonyx)

Getting started:
The audio engine starts on its own after install and at every login.
Open the app with fonyx.ui, or use the CLI: fonyx group ls

Requires a PipeWire session with WirePlumber.

Source and docs: https://github.com/ArdenDiago/Fonyx
License: GPL-3.0-or-later
```

The snap is strictly confined. The daemon is a snap user service
(`snap.fonyx.daemon`, `daemon-scope: user`), so it starts at install and at
login without the systemd units in `packaging/systemd/`. It reaches PipeWire
through the `pipewire` interface, which is privileged: until the store grants
auto-connect, users run `sudo snap connect fonyx:pipewire` once. Config lives
in `~/snap/fonyx/current/.config/fonyx/`, not `~/.config/fonyx/`. `fonyx
system` and the Settings tab's autostart toggles drive `systemctl --user`
units that do not exist in the snap; use `snap start|stop|restart --user
fonyx.daemon` there instead.

## Limitations

- No release artifacts or version tags.
