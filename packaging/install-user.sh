#!/bin/sh
# Installs the daemon as a systemd *user* service without root: starts at
# login, restarts on crash or hang (see the unit's watchdog). Re-run after
# pulling changes to install the new build.
set -eu
cd "$(dirname "$0")/.."

cargo build --release -p mtxpanel-daemon -p mtxpanel-ui

# Copy then rename: overwriting a running binary in place fails (ETXTBSY).
mkdir -p "$HOME/.local/bin"
install -m755 target/release/mtxpanel-daemon "$HOME/.local/bin/mtxpanel-daemon.new"
mv -f "$HOME/.local/bin/mtxpanel-daemon.new" "$HOME/.local/bin/mtxpanel-daemon"
install -m755 target/release/mtxpanel-ui "$HOME/.local/bin/mtxpanel-ui.new"
mv -f "$HOME/.local/bin/mtxpanel-ui.new" "$HOME/.local/bin/mtxpanel-ui"

unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
mkdir -p "$unit_dir"
sed 's|^ExecStart=/usr/bin/mtxpanel-daemon|ExecStart=%h/.local/bin/mtxpanel-daemon|' \
    packaging/systemd/mtxpanel-daemon.service > "$unit_dir/mtxpanel-daemon.service"
# The Tauri UI is what opens at login.
sed 's|^ExecStart=/usr/bin/mtxpanel-ui|ExecStart=%h/.local/bin/mtxpanel-ui|' \
    packaging/systemd/mtxpanel-ui.service > "$unit_dir/mtxpanel-ui.service"

# Quickshell files + its unit (the packaged unit expects /usr/share/mtxpanel).
# Not enabled here (Settings tab toggle). MtxPanelPopup.qml is the fader
# popup meant for a compositor keybind, e.g. niri:
#   Mod+A { spawn-sh "pkill -f MtxPanelPopup.qml || quickshell -p ~/.local/share/mtxpanel/quickshell/MtxPanelPopup.qml"; }
share="${XDG_DATA_HOME:-$HOME/.local/share}/mtxpanel/quickshell"
rm -rf "$share"
mkdir -p "$share"
cp -r quickshell/. "$share/"
sed "s|/usr/share/mtxpanel/quickshell|$share|" \
    packaging/systemd/mtxpanel-quickshell.service > "$unit_dir/mtxpanel-quickshell.service"

# A hand-started daemon would hold the D-Bus name and block the service.
if ! systemctl --user is-active --quiet mtxpanel-daemon.service; then
    pkill -x mtxpanel-daemon || true
fi

systemctl --user daemon-reload
systemctl --user enable mtxpanel-daemon.service mtxpanel-ui.service
# Clear a start-limit left by earlier failed attempts, then (re)start.
systemctl --user reset-failed mtxpanel-daemon.service mtxpanel-quickshell.service 2>/dev/null || true
systemctl --user restart mtxpanel-daemon.service
systemctl --user try-restart mtxpanel-quickshell.service
systemctl --user --no-pager status mtxpanel-daemon.service | head -n 5
