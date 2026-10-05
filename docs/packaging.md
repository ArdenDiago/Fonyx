# Packaging

SonicForge has no published packages and no tagged release. It is installed
from a source checkout with the files in `packaging/`:

| Path | What it is |
|---|---|
| `install-user.sh` | Per-user install from a source checkout |
| `systemd/` | Three systemd user units, used by the install script |

## User install

```bash
sh packaging/install-user.sh
```

The script needs no root. It:

1. Builds release binaries: `cargo build --release -p mtxpanel-daemon -p mtxpanel-ui`.
2. Installs `mtxpanel-daemon` and `mtxpanel-ui` into `~/.local/bin`. Each is
   copied under a temporary name and renamed, because overwriting a running
   binary in place fails.
3. Writes the daemon and UI units into `~/.config/systemd/user/`, rewriting
   `ExecStart` from `/usr/bin/...` to `~/.local/bin/...`.
4. Copies `quickshell/` to `~/.local/share/mtxpanel/quickshell/` and writes
   the Quickshell unit pointing there.
5. Stops any hand-started daemon that would hold the D-Bus name.
6. Reloads systemd, enables the daemon and UI units, restarts the daemon,
   and restarts the UI and Quickshell units if they are running.

The Quickshell unit is installed but not enabled; the Settings tab toggles
it.

Re-run the script after pulling changes to install the new build. Restarting
the daemon interrupts audio briefly while streams are re-routed.

To remove the install, disable the units and delete the files the script
created:

```bash
systemctl --user disable --now mtxpanel-ui mtxpanel-quickshell mtxpanel-daemon
rm ~/.local/bin/mtxpanel-daemon ~/.local/bin/mtxpanel-ui
rm ~/.config/systemd/user/mtxpanel-*.service
rm -r ~/.local/share/mtxpanel
systemctl --user daemon-reload
```

Configuration in `~/.config/mtxpanel/` is left in place.

## systemd units

All three are **user** units.

### `mtxpanel-daemon.service`

| Setting | Value | Reason |
|---|---|---|
| `Type` | `notify` | The daemon reports `READY=1` once PipeWire is connected |
| `WatchdogSec` | `10` | The daemon pings every 2 s from its engine thread; a hang is treated like a crash |
| `Restart` | `on-failure`, after 1 s | Recover from crashes and watchdog kills |
| `StartLimitBurst` | 5 in 60 s | Stop a crash loop |
| `After` / `Wants` | `pipewire.service` | Start after PipeWire |
| `WantedBy` | `default.target` | Start at login |

### `mtxpanel-ui.service`

Starts `mtxpanel-ui` with the graphical session, after the daemon.
`Restart=on-failure`.

### `mtxpanel-quickshell.service`

Runs `quickshell -p .../MtxPanelApp.qml` with the graphical session. It
starts the standalone app, not the widget, which has no window of its own.

The packaged units use `/usr/bin` and `/usr/share/mtxpanel` paths, as a
system package would. The install script rewrites them for a per-user
install.

### Autostart toggles

The Settings tab's two autostart switches call
`systemctl --user enable|disable` on the daemon and Quickshell units. They
work only once the unit files are installed.

## Limitations

- No release artifacts or version tags.
- The application icon is a placeholder.
