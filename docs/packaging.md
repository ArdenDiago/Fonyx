# Packaging

SonicForge has no published packages and no tagged release. It is installed
from a source checkout with the files in `packaging/`:

| Path | What it is |
|---|---|
| `install-user.sh` | Per-user install from a source checkout |
| `systemd/` | Three systemd user units, used by the install script |
| `branding/` | The logo artwork and the script that cuts the icons out of it |

## User install

```bash
git clone https://github.com/ArdenDiago/SonicForge.git
cd SonicForge
sh packaging/install-user.sh
```

The script needs no root. It:

1. Builds release binaries: `cargo build --release -p sonicforge-daemon -p sonicforge-ui -p sonicforge-cli`.
2. Installs `sonicforge-daemon`, `sonicforge-ui` and the `sonicforge` CLI
   ([Command line](cli.md)) into `~/.local/bin`. Each is
   copied under a temporary name and renamed, because overwriting a running
   binary in place fails.
3. Installs a launcher entry and the app icon under `~/.local/share`
   (`applications/org.sonicforge.app.desktop` and
   `icons/hicolor/512x512/apps/org.sonicforge.App.png`).
4. Writes the daemon and UI units into `~/.config/systemd/user/`, rewriting
   `ExecStart` from `/usr/bin/...` to `~/.local/bin/...`.
5. Copies `quickshell/` to `~/.local/share/sonicforge/quickshell/` and writes
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
systemctl --user disable --now sonicforge-ui sonicforge-quickshell sonicforge-daemon
rm ~/.local/bin/sonicforge-daemon ~/.local/bin/sonicforge-ui ~/.local/bin/sonicforge
rm ~/.config/systemd/user/sonicforge-*.service
rm -r ~/.local/share/sonicforge
rm ~/.local/share/applications/org.sonicforge.app.desktop
rm ~/.local/share/icons/hicolor/512x512/apps/org.sonicforge.App.png
systemctl --user daemon-reload
```

Configuration in `~/.config/sonicforge/` is left in place.

## systemd units

All three are **user** units.

### `sonicforge-daemon.service`

| Setting | Value | Reason |
|---|---|---|
| `Type` | `notify` | The daemon reports `READY=1` once PipeWire is connected |
| `WatchdogSec` | `10` | The daemon pings every 2 s from its engine thread; a hang is treated like a crash |
| `Restart` | `on-failure`, after 1 s | Recover from crashes and watchdog kills |
| `StartLimitBurst` | 5 in 60 s | Stop a crash loop |
| `After` / `Wants` | `pipewire.service` | Start after PipeWire |
| `WantedBy` | `default.target` | Start at login |

### `sonicforge-ui.service`

Starts `sonicforge-ui` with the graphical session, after the daemon.
`Restart=on-failure`.

### `sonicforge-quickshell.service`

Runs `quickshell -p .../SonicForgeApp.qml` with the graphical session. It
starts the standalone app, not the widget, which has no window of its own.

The packaged units use `/usr/bin` and `/usr/share/sonicforge` paths, as a
system package would. The install script rewrites them for a per-user
install.

### Autostart toggles

The Settings tab's two autostart switches call
`systemctl --user enable|disable` on the daemon and Quickshell units. They
work only once the unit files are installed.

## Limitations

- No release artifacts or version tags.
